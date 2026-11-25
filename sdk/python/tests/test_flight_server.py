"""End-to-end tests of FlightClient against an in-process ``pyarrow.flight`` server.

The server mimics the DCH Flight service contract: Flight SQL commands arrive
as ``google.protobuf.Any`` in ``GetFlightInfo``, each answer has one endpoint
with a ticket and no locations, ``GetSupportedConnectors`` is a ``DoAction``,
and every call must carry ``authorization`` and ``x-tenant-id`` headers.  No
mocks: this exercises the real gRPC client, headers, deadlines and errors.
"""

from __future__ import annotations

import json
import threading
import time
from collections.abc import Iterator
from typing import Any

import pyarrow as pa
import pyarrow.flight as flight
import pytest

from data_connect_hub._flight import FlightClient
from data_connect_hub.exceptions import DCHConnectionError, DCHQueryError, DCHTimeoutError

_TENANT = "tenant-a"
_TYPE_PREFIX = "type.googleapis.com/arrow.flight.protocol.sql."


def _read_varint(data: bytes, offset: int) -> tuple[int, int]:
    result = shift = 0
    while True:
        b = data[offset]
        offset += 1
        result |= (b & 0x7F) << shift
        if not b & 0x80:
            return result, offset
        shift += 7


def _fields(data: bytes) -> dict[int, Any]:
    out: dict[int, Any] = {}
    i = 0
    while i < len(data):
        tag, i = _read_varint(data, i)
        if tag & 7 == 0:
            out[tag >> 3], i = _read_varint(data, i)
        else:
            length, i = _read_varint(data, i)
            out[tag >> 3] = data[i : i + length]
            i += length
    return out


def _decode_command(raw: bytes) -> tuple[str, dict[int, Any]]:
    outer = _fields(raw)
    return outer[1].decode().removeprefix(_TYPE_PREFIX), _fields(outer.get(2, b""))


def _sql_info_table(requested: list[int]) -> pa.Table:
    known = {0: "Data Connect Hub", 1: "9.9.9", 2: "1.3"}
    ids = [i for i in requested if i in known]
    value = pa.UnionArray.from_dense(
        pa.array([0] * len(ids), type=pa.int8()),
        pa.array(range(len(ids)), type=pa.int32()),
        [pa.array([known[i] for i in ids]), pa.array([], type=pa.bool_())],
        ["string_value", "bool_value"],
        [0, 1],
    )
    return pa.table({"info_name": pa.array(ids, type=pa.uint32()), "value": value})


class _HeaderCheck(flight.ServerMiddleware):  # type: ignore[misc]
    def __init__(self, headers: dict[str, list[str]]) -> None:
        self.headers = headers


class _AuthFactory(flight.ServerMiddlewareFactory):  # type: ignore[misc]
    def __init__(self, server: _DCHLikeServer) -> None:
        self._server = server

    def start_call(self, info: Any, headers: dict[str, list[str]]) -> _HeaderCheck:
        token = (headers.get("authorization") or [""])[0]
        if token not in self._server.valid_tokens:
            raise flight.FlightUnauthenticatedError("invalid bearer token")
        if (headers.get("x-tenant-id") or [""])[0] != _TENANT:
            raise flight.FlightUnauthorizedError("permission denied for tenant")
        self._server.seen_headers.append(headers)
        return _HeaderCheck(headers)


class _DCHLikeServer(flight.FlightServerBase):  # type: ignore[misc]
    def __init__(self) -> None:
        super().__init__("grpc://127.0.0.1:0", middleware={"auth": _AuthFactory(self)})
        self.valid_tokens = {"Bearer good"}
        self.seen_headers: list[dict[str, list[str]]] = []
        self.commands: list[tuple[str, dict[int, Any]]] = []
        self.tables: dict[str, pa.Table] = {}
        self.endpoints_per_query = 1
        self.delay = 0.0
        self.fail_with: Exception | None = None
        self.stream_cancelled = threading.Event()
        self.stream_completed = threading.Event()
        self.stream_finished = threading.Event()

    # The DCH server answers GetFlightInfo with endpoints whose ticket the client
    # sends back in DoGet; here the ticket is "<kind>:<payload>[:<part>]".
    def get_flight_info(self, context: Any, descriptor: Any) -> flight.FlightInfo:
        if self.delay:
            time.sleep(self.delay)
        if self.fail_with is not None:
            raise self.fail_with
        kind, fields = _decode_command(descriptor.command)
        self.commands.append((kind, fields))
        if kind == "CommandStatementQuery":
            sql = fields[1].decode()
            schema = self.tables[sql].schema
            tickets = [f"sql:{sql}:{part}".encode() for part in range(self.endpoints_per_query)]
        elif kind == "CommandGetSqlInfo":
            ids = list(fields.get(1, b""))
            schema = _sql_info_table(ids).schema
            tickets = [b"info:" + bytes(ids)]
        elif kind == "CommandGetTables":
            schema = pa.schema([("table_name", pa.string())])
            tickets = [
                b"tables:"
                + json.dumps({str(k): v if isinstance(v, int) else v.decode() for k, v in fields.items()}).encode()
            ]
        else:
            raise flight.FlightServerError(f"unsupported command {kind}")
        endpoints = [flight.FlightEndpoint(t, []) for t in tickets]
        return flight.FlightInfo(schema, descriptor, endpoints, -1, -1)

    def do_get(self, context: Any, ticket: Any) -> flight.FlightDataStream:
        kind, _, payload = ticket.ticket.decode().partition(":")
        if kind == "sql":
            sql, _, part = payload.rpartition(":")
            table = self.tables[sql]
            if self.endpoints_per_query > 1:
                table = table.slice(int(part), 1)
            if sql.startswith("SELECT slow"):
                return flight.GeneratorStream(table.schema, self._slow_batches(context, table))
            return flight.RecordBatchStream(table)
        if kind == "info":
            return flight.RecordBatchStream(_sql_info_table(list(payload.encode())))
        if kind == "tables":
            return flight.RecordBatchStream(pa.table({"table_name": [payload]}))
        raise flight.FlightServerError("bad ticket")

    def _slow_batches(self, context: Any, table: pa.Table) -> Iterator[pa.RecordBatch]:
        try:
            for batch in table.to_batches(max_chunksize=1):
                if context.is_cancelled():
                    self.stream_cancelled.set()
                    return
                yield batch
                time.sleep(0.05)
            self.stream_completed.set()
        finally:
            self.stream_finished.set()

    def do_action(self, context: Any, action: Any) -> Iterator[flight.Result]:
        if action.type != "GetSupportedConnectors":
            raise flight.FlightServerError("unknown action")
        connectors = pa.table({"name": ["postgres", "sqlite"], "description": ["", ""]})
        sink = pa.BufferOutputStream()
        with pa.ipc.new_stream(sink, connectors.schema) as writer:
            writer.write_table(connectors)
        yield flight.Result(sink.getvalue())


@pytest.fixture()
def server() -> Iterator[_DCHLikeServer]:
    srv = _DCHLikeServer()
    srv.tables = {
        "SELECT * FROM cities": pa.table({"id": [1, 2, 3], "name": ["Oslo", "Lima", "Pune"]}),
        "SELECT slow": pa.table({"n": list(range(50))}),
    }
    try:
        yield srv
    finally:
        srv.shutdown()


def _client(server: _DCHLikeServer, **kwargs: Any) -> FlightClient:
    kwargs.setdefault("token", "good")
    kwargs.setdefault("tenant_id", _TENANT)
    return FlightClient(url=f"grpc://127.0.0.1:{server.port}", **kwargs)


class TestQueries:
    def test_read(self, server: _DCHLikeServer) -> None:
        table = _client(server).read("SELECT * FROM cities", "conn-1")
        assert table.equals(server.tables["SELECT * FROM cities"])
        assert server.commands == [("CommandStatementQuery", {1: b"SELECT * FROM cities"})]
        # GetFlightInfo and DoGet both carry the data-connection header.
        assert [h["x-data-connection-id"] for h in server.seen_headers] == [["conn-1"], ["conn-1"]]

    def test_read_multiple_endpoints(self, server: _DCHLikeServer) -> None:
        server.endpoints_per_query = 3
        table = _client(server).read("SELECT * FROM cities", "conn-1")
        assert table.column("name").to_pylist() == ["Oslo", "Lima", "Pune"]

    def test_read_pandas(self, server: _DCHLikeServer) -> None:
        df = _client(server).read_pandas("SELECT * FROM cities", "conn-1")
        assert list(df["name"]) == ["Oslo", "Lima", "Pune"]

    def test_read_batches(self, server: _DCHLikeServer) -> None:
        batches = list(_client(server).read_batches("SELECT slow", "conn-1"))
        assert sum(b.num_rows for b in batches) == 50

    def test_read_batches_early_close_stops_server_stream(self, server: _DCHLikeServer) -> None:
        stream = _client(server).read_batches("SELECT slow", "conn-1")
        next(stream)
        started = time.monotonic()
        stream.close()
        # 50 rows at 50 ms each would take 2.5 s if the stream ran to completion.
        assert server.stream_finished.wait(5)
        assert not server.stream_completed.is_set(), "server streamed every row; the call was not cancelled"
        assert time.monotonic() - started < 2.0


class TestMetadata:
    def test_server_info(self, server: _DCHLikeServer) -> None:
        info = _client(server).server_info()
        assert info["vendor_name"] == "Data Connect Hub"
        assert info["vendor_version"] == "9.9.9"
        assert info["vendor_arrow_version"] == "1.3"
        assert info["driver_arrow_version"] == pa.__version__
        assert info["supported_connectors"] == ["postgres", "sqlite"]
        assert server.commands == [("CommandGetSqlInfo", {1: b"\x00\x01\x02"})]
        assert all("x-data-connection-id" not in h for h in server.seen_headers)

    def test_get_tables(self, server: _DCHLikeServer) -> None:
        table = _client(server).get_tables("conn-1", table_name_filter="cit%", include_schema=True)
        assert json.loads(table.column("table_name")[0].as_py()) == {"3": "cit%", "5": 1}


class TestAuthAndErrors:
    def test_expired_token_is_refreshed_once(self, server: _DCHLikeServer) -> None:
        tokens = iter(["expired", "good"])
        client = _client(server, token="", token_provider=lambda: next(tokens))
        assert client.read("SELECT * FROM cities", "conn-1").num_rows == 3

    def test_invalid_static_token(self, server: _DCHLikeServer) -> None:
        with pytest.raises(DCHConnectionError, match=r"(?i)unauthenticated"):
            _client(server, token="bad").server_info()

    def test_wrong_tenant(self, server: _DCHLikeServer) -> None:
        with pytest.raises(DCHConnectionError, match=r"(?i)unauthorized|permission.?denied"):
            _client(server, tenant_id="tenant-b").read("SELECT * FROM cities", "conn-1")

    def test_query_rejected(self, server: _DCHLikeServer) -> None:
        server.fail_with = pa.ArrowInvalid("read-only: only SELECT statements are allowed")
        with pytest.raises(DCHQueryError, match="read-only"):
            _client(server).read("DROP TABLE cities", "conn-1")

    def test_deadline(self, server: _DCHLikeServer) -> None:
        server.delay = 1.0
        with pytest.raises(DCHTimeoutError):
            _client(server, timeout=0.2).read("SELECT * FROM cities", "conn-1")

    def test_unreachable_server(self) -> None:
        client = FlightClient(url="grpc://127.0.0.1:1", token="good", tenant_id=_TENANT, timeout=2)
        with pytest.raises((DCHConnectionError, DCHTimeoutError)):
            client.read("SELECT 1", "conn-1")
