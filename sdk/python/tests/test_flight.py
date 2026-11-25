"""Unit tests for the pyarrow.flight-based FlightClient.

``flight.connect`` is replaced by a fake client, but ``FlightCallOptions`` is the
real class (wrapped only to record its arguments), so the minimum-dependency CI
job catches pyarrow APIs missing from old releases.  ``test_flight_server.py``
covers the same paths end to end against an in-process Flight server.
"""

from __future__ import annotations

import os
import sys
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from types import SimpleNamespace
from typing import Any
from unittest.mock import MagicMock, patch

import pandas as pd
import pyarrow as pa
import pyarrow.flight as flight
import pytest

from data_connect_hub._flight import FlightClient, _grpc_proxy_options, _map_error
from data_connect_hub.client import _build_urls
from data_connect_hub.exceptions import (
    DCHConfigError,
    DCHConnectionError,
    DCHError,
    DCHQueryError,
    DCHTimeoutError,
)

from .conftest import unpack_any

_URL = "grpc://localhost:50051"
_STATEMENT_TYPE_URL = "type.googleapis.com/arrow.flight.protocol.sql.CommandStatementQuery"
_SQL_INFO_TYPE_URL = "type.googleapis.com/arrow.flight.protocol.sql.CommandGetSqlInfo"
_GET_TABLES_TYPE_URL = "type.googleapis.com/arrow.flight.protocol.sql.CommandGetTables"


# -- fake pyarrow Flight client -------------------------------------------------


class _FakeReader:
    """Stand-in for ``FlightStreamReader``: ``read_all``, ``read_chunk`` and ``cancel``."""

    def __init__(self, table: pa.Table, *, error: Exception | None = None, error_after: int = 0) -> None:
        self.table = table
        self._batches = table.to_batches()
        self._error = error
        self._error_after = error_after
        self._pos = 0
        self.cancel = MagicMock()

    def read_all(self) -> pa.Table:
        if self._error is not None:
            raise self._error
        return self.table

    def read_chunk(self) -> SimpleNamespace:
        if self._error is not None and self._pos >= self._error_after:
            raise self._error
        if self._pos >= len(self._batches):
            raise StopIteration
        batch = self._batches[self._pos]
        self._pos += 1
        return SimpleNamespace(data=batch, app_metadata=None)


def _endpoint(ticket: bytes = b"ticket-0", locations: list[str] | None = None) -> SimpleNamespace:
    return SimpleNamespace(
        ticket=flight.Ticket(ticket),
        locations=[SimpleNamespace(uri=uri.encode()) for uri in (locations or [])],
    )


def _fake_client(
    *results: pa.Table | _FakeReader,
    schema: pa.Schema | None = None,
    locations: list[str] | None = None,
) -> MagicMock:
    """Build a fake ``flight.FlightClient`` returning one endpoint per result."""
    readers = [r if isinstance(r, _FakeReader) else _FakeReader(r) for r in results]
    client = MagicMock()
    info = MagicMock()
    info.endpoints = [_endpoint(f"ticket-{i}".encode(), locations) for i in range(len(readers))]
    info.schema = schema if schema is not None else (readers[0].table.schema if readers else pa.schema([]))
    client.get_flight_info.return_value = info
    client.do_get.side_effect = readers
    return client


def _ipc_bytes(table: pa.Table) -> bytes:
    sink = pa.BufferOutputStream()
    with pa.ipc.new_stream(sink, table.schema) as writer:
        writer.write_table(table)
    return bytes(sink.getvalue().to_pybytes())


def _sql_info_table(values: dict[int, str]) -> pa.Table:
    """Build a GetSqlInfo result: ``info_name`` uint32 and a dense-union ``value``."""
    strings = pa.array(list(values.values()), type=pa.string())
    bools = pa.array([], type=pa.bool_())
    value = pa.UnionArray.from_dense(
        pa.array([0] * len(values), type=pa.int8()),
        pa.array(range(len(values)), type=pa.int32()),
        [strings, bools],
        ["string_value", "bool_value"],
        [0, 1],
    )
    return pa.table({"info_name": pa.array(list(values), type=pa.uint32()), "value": value})


def _server_info_client(values: dict[int, str] | None = None, connectors: list[str] | None = None) -> MagicMock:
    values = values if values is not None else {0: "Data Connect Hub", 1: "0.4.0", 2: "1.3"}
    client = _fake_client(_sql_info_table(values))
    names = connectors if connectors is not None else ["postgres"]
    connectors_table = pa.table({"name": names, "description": [""] * len(names)})
    client.do_action.return_value = [SimpleNamespace(body=pa.py_buffer(_ipc_bytes(connectors_table)))]
    return client


@contextmanager
def _patched(*clients: MagicMock) -> Iterator[tuple[MagicMock, MagicMock]]:
    """Patch ``flight.connect`` to hand out *clients* in order; record ``FlightCallOptions`` calls."""
    with (
        patch("data_connect_hub._flight.flight.connect", side_effect=list(clients)) as connect,
        patch("data_connect_hub._flight.flight.FlightCallOptions", wraps=flight.FlightCallOptions) as call_options,
    ):
        yield connect, call_options


def _sent_command(client: MagicMock) -> tuple[str, dict[int, bytes | int]]:
    descriptor = client.get_flight_info.call_args.args[0]
    return unpack_any(descriptor.command)


def _sent_headers(call_options: MagicMock, index: int = -1) -> dict[bytes, bytes]:
    return dict(call_options.call_args_list[index].kwargs["headers"])


def _counting_provider() -> tuple[Callable[[], str], Callable[[], int]]:
    calls = 0

    def provider() -> str:
        nonlocal calls
        calls += 1
        return f"token-{calls}"

    return provider, lambda: calls


@pytest.fixture()
def flight_client() -> FlightClient:
    return FlightClient(url=_URL, token="tok", tenant_id="t1")


# -- tests ----------------------------------------------------------------------


class TestTenantValidation:
    @pytest.mark.parametrize(
        "operation",
        [
            lambda c: c.read("SELECT 1", "conn-1"),
            lambda c: list(c.read_batches("SELECT 1", "conn-1")),
            lambda c: c.read_pandas("SELECT 1", "conn-1"),
            lambda c: c.server_info(),
            lambda c: c.get_tables("conn-1"),
        ],
        ids=["read", "read_batches", "read_pandas", "server_info", "get_tables"],
    )
    def test_missing_tenant_rejected_before_flight_request(self, operation: Callable[[FlightClient], Any]) -> None:
        client = FlightClient(url=_URL, token="tok")
        with _patched() as (connect, _), pytest.raises(DCHConfigError, match="tenant_id must be provided"):
            operation(client)
        connect.assert_not_called()


class TestRead:
    def test_returns_table_and_sends_statement_query(self, flight_client: FlightClient) -> None:
        table = pa.table({"col": [1, 2, 3]})
        client = _fake_client(table)
        with _patched(client):
            result = flight_client.read("SELECT * FROM t", "conn-1")

        assert result is table
        type_url, fields = _sent_command(client)
        assert type_url == _STATEMENT_TYPE_URL
        assert fields == {1: b"SELECT * FROM t"}
        opts = client.get_flight_info.call_args.args[1]
        assert client.do_get.call_args.args == (flight.Ticket(b"ticket-0"), opts)
        client.close.assert_called_once()

    def test_empty_result_keeps_schema(self, flight_client: FlightClient) -> None:
        empty = pa.table({"col": pa.array([], type=pa.int64())})
        with _patched(_fake_client(empty)):
            result = flight_client.read("SELECT 1", "conn-1")
        assert result.num_rows == 0
        assert result.schema == empty.schema

    def test_no_endpoints_returns_empty_table_with_flight_info_schema(self, flight_client: FlightClient) -> None:
        schema = pa.schema([("col", pa.int64())])
        client = _fake_client(schema=schema)
        with _patched(client):
            result = flight_client.read("SELECT 1", "conn-1")
        assert result.num_rows == 0
        assert result.schema == schema
        client.do_get.assert_not_called()

    def test_multiple_endpoints_are_concatenated_in_order(self, flight_client: FlightClient) -> None:
        client = _fake_client(pa.table({"col": [1, 2]}), pa.table({"col": [3]}))
        with _patched(client):
            result = flight_client.read("SELECT 1", "conn-1")
        assert result.column("col").to_pylist() == [1, 2, 3]
        assert [c.args[0] for c in client.do_get.call_args_list] == [
            flight.Ticket(b"ticket-0"),
            flight.Ticket(b"ticket-1"),
        ]

    def test_reuse_connection_location_is_accepted(self, flight_client: FlightClient) -> None:
        table = pa.table({"col": [1]})
        with _patched(_fake_client(table, locations=["arrow-flight-reuse-connection://?"])):
            assert flight_client.read("SELECT 1", "conn-1") is table

    def test_endpoint_on_other_server_is_rejected(self, flight_client: FlightClient) -> None:
        client = _fake_client(pa.table({"col": [1]}), locations=["grpc+tls://elsewhere:443"])
        with _patched(client), pytest.raises(DCHQueryError, match="another location"):
            flight_client.read("SELECT 1", "conn-1")
        client.do_get.assert_not_called()
        client.close.assert_called_once()

    @pytest.mark.parametrize(
        ("raised", "expected"),
        [
            (pa.ArrowInvalid("read-only: only SELECT is allowed"), DCHQueryError),
            (flight.FlightServerError("boom"), DCHQueryError),
            (flight.FlightInternalError("internal"), DCHQueryError),
            (pa.ArrowNotImplementedError("unimplemented"), DCHQueryError),
            (flight.FlightUnavailableError("unreachable"), DCHConnectionError),
            (flight.FlightUnauthorizedError("permission denied"), DCHConnectionError),
            (flight.FlightUnauthenticatedError("unauthenticated"), DCHConnectionError),
            (flight.FlightTimedOutError("deadline exceeded"), DCHTimeoutError),
        ],
    )
    @pytest.mark.parametrize("failure_point", ["get_flight_info", "do_get"])
    def test_errors_are_mapped_and_client_closed(
        self, flight_client: FlightClient, failure_point: str, raised: Exception, expected: type[DCHError]
    ) -> None:
        client = _fake_client(pa.table({"col": [1]}))
        getattr(client, failure_point).side_effect = raised
        with _patched(client), pytest.raises(expected, match=str(raised)) as exc_info:
            flight_client.read("SELECT 1", "conn-1")
        assert exc_info.value.__cause__ is raised
        client.close.assert_called_once()

    def test_connect_failure_mapped(self, flight_client: FlightClient) -> None:
        with (
            patch("data_connect_hub._flight.flight.connect", side_effect=pa.ArrowInvalid("bad uri")),
            pytest.raises(DCHConnectionError, match="bad uri"),
        ):
            flight_client.read("SELECT 1", "conn-1")


class TestReadPandas:
    def test_returns_dataframe(self, flight_client: FlightClient) -> None:
        client = _fake_client(pa.table({"col": [1, 2, 3]}))
        with _patched(client):
            result = flight_client.read_pandas("SELECT 1", "conn-1")
        assert isinstance(result, pd.DataFrame)
        assert list(result["col"]) == [1, 2, 3]
        client.close.assert_called_once()

    def test_empty_result_returns_empty_dataframe(self, flight_client: FlightClient) -> None:
        with _patched(_fake_client(pa.table({"col": pa.array([], type=pa.int64())}))):
            result = flight_client.read_pandas("SELECT 1", "conn-1")
        assert isinstance(result, pd.DataFrame)
        assert len(result) == 0


class TestReadBatches:
    def _batched(self) -> pa.Table:
        return pa.Table.from_batches(
            [
                pa.RecordBatch.from_pydict({"col": [1, 2]}),
                pa.RecordBatch.from_pydict({"col": [3]}),
                pa.RecordBatch.from_pydict({"col": [4]}),
            ]
        )

    def test_yields_batches_and_closes(self, flight_client: FlightClient) -> None:
        reader = _FakeReader(self._batched())
        client = _fake_client(reader)
        with _patched(client):
            batches = list(flight_client.read_batches("SELECT 1", "conn-1"))

        assert [b.num_rows for b in batches] == [2, 1, 1]
        assert pa.Table.from_batches(batches).column("col").to_pylist() == [1, 2, 3, 4]
        _, fields = _sent_command(client)
        assert fields == {1: b"SELECT 1"}
        reader.cancel.assert_not_called()
        client.close.assert_called_once()

    def test_lazy_until_iterated(self, flight_client: FlightClient) -> None:
        with _patched(_fake_client(pa.table({"col": [1]}))) as (connect, _):
            stream = flight_client.read_batches("SELECT 1", "conn-1")
            connect.assert_not_called()
            assert len(list(stream)) == 1
            connect.assert_called_once()

    def test_early_close_cancels_stream_and_closes(self, flight_client: FlightClient) -> None:
        reader = _FakeReader(self._batched())
        client = _fake_client(reader)
        with _patched(client):
            stream = flight_client.read_batches("SELECT 1", "conn-1")
            first = next(stream)
            stream.close()
        assert first.num_rows == 2
        reader.cancel.assert_called_once()
        client.close.assert_called_once()

    def test_metadata_only_chunks_are_skipped(self, flight_client: FlightClient) -> None:
        batch = pa.RecordBatch.from_pydict({"col": [1]})
        reader = MagicMock()
        reader.read_chunk.side_effect = [
            SimpleNamespace(data=None, app_metadata=pa.py_buffer(b"progress")),
            SimpleNamespace(data=batch, app_metadata=None),
            StopIteration,
        ]
        client = _fake_client(pa.table({"col": [1]}))
        client.do_get.side_effect = [reader]
        with _patched(client):
            assert list(flight_client.read_batches("SELECT 1", "conn-1")) == [batch]
        reader.cancel.assert_not_called()

    def test_multiple_endpoints_streamed_in_order(self, flight_client: FlightClient) -> None:
        with _patched(_fake_client(pa.table({"col": [1]}), pa.table({"col": [2]}))):
            batches = list(flight_client.read_batches("SELECT 1", "conn-1"))
        assert [b.column(0).to_pylist() for b in batches] == [[1], [2]]

    def test_empty_result(self, flight_client: FlightClient) -> None:
        client = _fake_client(pa.table({"col": pa.array([], type=pa.int64())}))
        with _patched(client):
            assert list(flight_client.read_batches("SELECT 1", "conn-1")) == []
        client.close.assert_called_once()

    def test_query_error_mapped_and_closes(self, flight_client: FlightClient) -> None:
        client = _fake_client(pa.table({"col": [1]}))
        client.get_flight_info.side_effect = pa.ArrowInvalid("bad sql")
        with _patched(client), pytest.raises(DCHQueryError, match="bad sql"):
            list(flight_client.read_batches("BAD SQL", "conn-1"))
        client.close.assert_called_once()

    def test_connection_error_mapped(self, flight_client: FlightClient) -> None:
        client = _fake_client(pa.table({"col": [1]}))
        client.get_flight_info.side_effect = flight.FlightUnavailableError("unreachable")
        with _patched(client), pytest.raises(DCHConnectionError, match="unreachable"):
            list(flight_client.read_batches("SELECT 1", "conn-1"))

    def test_mid_stream_error_mapped_cancels_and_closes(self, flight_client: FlightClient) -> None:
        reader = _FakeReader(self._batched(), error=flight.FlightInternalError("stream broke"), error_after=1)
        client = _fake_client(reader)
        received = []
        with _patched(client), pytest.raises(DCHQueryError, match="stream broke"):
            received.extend(flight_client.read_batches("SELECT 1", "conn-1"))
        assert len(received) == 1
        reader.cancel.assert_called_once()
        client.close.assert_called_once()

    @pytest.mark.parametrize("failure_point", ["get_flight_info", "do_get"])
    def test_auth_error_before_first_batch_refreshes_and_retries(self, failure_point: str) -> None:
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        rejected = _fake_client(pa.table({"col": [1]}))
        getattr(rejected, failure_point).side_effect = flight.FlightUnauthenticatedError("token expired")
        accepted = _fake_client(pa.table({"col": [1]}))
        with _patched(rejected, accepted) as (_, call_options):
            batches = list(client.read_batches("SELECT 1", "conn-1"))

        assert len(batches) == 1
        assert calls() == 2
        assert [_sent_headers(call_options, i)[b"authorization"] for i in range(2)] == [
            b"Bearer token-1",
            b"Bearer token-2",
        ]
        rejected.close.assert_called_once()
        accepted.close.assert_called_once()

    def test_auth_error_after_first_batch_not_retried(self) -> None:
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        reader = _FakeReader(self._batched(), error=flight.FlightUnauthenticatedError("expired"), error_after=1)
        with _patched(_fake_client(reader)), pytest.raises(DCHConnectionError, match="expired"):
            list(client.read_batches("SELECT 1", "conn-1"))
        assert calls() == 1

    def test_auth_error_retried_only_once(self) -> None:
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        first, second = _fake_client(), _fake_client()
        for fake in (first, second):
            fake.get_flight_info.side_effect = flight.FlightUnauthenticatedError("invalid token")
        with _patched(first, second), pytest.raises(DCHConnectionError, match="invalid token"):
            list(client.read_batches("SELECT 1", "conn-1"))
        assert calls() == 2


class TestServerInfo:
    def test_returns_vendor_driver_and_connectors(self, flight_client: FlightClient) -> None:
        from data_connect_hub import __version__

        client = _server_info_client(connectors=["postgres", "sqlite"])
        with _patched(client) as (connect, call_options):
            result = flight_client.server_info()

        assert result == {
            "vendor_name": "Data Connect Hub",
            "vendor_version": "0.4.0",
            "vendor_arrow_version": "1.3",
            "driver_name": "Data Connect Hub Python SDK (pyarrow.flight)",
            "driver_version": __version__,
            "driver_arrow_version": pa.__version__,
            "supported_connectors": ["postgres", "sqlite"],
        }
        connect.assert_called_once()
        client.close.assert_called_once()
        assert b"x-data-connection-id" not in _sent_headers(call_options)
        action, opts = client.do_action.call_args.args
        assert action.type == "GetSupportedConnectors"
        assert opts is client.get_flight_info.call_args.args[1]

    def test_requests_vendor_sql_info_ids(self, flight_client: FlightClient) -> None:
        client = _server_info_client()
        with _patched(client):
            flight_client.server_info()
        type_url, fields = _sent_command(client)
        assert type_url == _SQL_INFO_TYPE_URL
        assert fields == {1: b"\x00\x01\x02"}

    def test_unrequested_sql_info_ids_ignored(self, flight_client: FlightClient) -> None:
        client = _server_info_client({0: "DCH", 1: "1.0", 2: "1.3", 500: "extra"})
        with _patched(client):
            result = flight_client.server_info()
        assert result["vendor_name"] == "DCH"
        assert "extra" not in result.values()

    def test_connectors_json_fallback(self, flight_client: FlightClient) -> None:
        client = _server_info_client()
        client.do_action.return_value = [SimpleNamespace(body=pa.py_buffer(b'[{"name": "s3"}, {"name": "uri"}]'))]
        with _patched(client):
            assert flight_client.server_info()["supported_connectors"] == ["s3", "uri"]

    def test_no_connectors_result(self, flight_client: FlightClient) -> None:
        client = _server_info_client()
        client.do_action.return_value = []
        with _patched(client):
            assert flight_client.server_info()["supported_connectors"] == []

    @pytest.mark.parametrize("failure_point", ["get_flight_info", "do_action"])
    def test_errors_map_to_connection_error(self, flight_client: FlightClient, failure_point: str) -> None:
        client = _server_info_client()
        getattr(client, failure_point).side_effect = flight.FlightServerError("action failed")
        with _patched(client), pytest.raises(DCHConnectionError, match="action failed"):
            flight_client.server_info()
        client.close.assert_called_once()

    def test_permission_denied_maps_to_connection_error(self, flight_client: FlightClient) -> None:
        client = _server_info_client()
        client.get_flight_info.side_effect = flight.FlightUnauthorizedError("Flight returned unauthorized error")
        with _patched(client), pytest.raises(DCHConnectionError, match=r"(?i)unauthorized"):
            flight_client.server_info()


class TestHeaders:
    @pytest.mark.parametrize(
        "operation",
        [
            lambda c: c.read("SELECT 1", "my-conn"),
            lambda c: list(c.read_batches("SELECT 1", "my-conn")),
            lambda c: c.get_tables("my-conn"),
        ],
        ids=["read", "read_batches", "get_tables"],
    )
    def test_data_calls_send_auth_tenant_and_connection_id(
        self, flight_client: FlightClient, operation: Callable[[FlightClient], Any]
    ) -> None:
        client = _fake_client(pa.table({"col": [1]}))
        with _patched(client) as (_, call_options):
            operation(flight_client)
        call_options.assert_called_once()
        assert sorted(call_options.call_args.kwargs["headers"]) == [
            (b"authorization", b"Bearer tok"),
            (b"x-data-connection-id", b"my-conn"),
            (b"x-tenant-id", b"t1"),
        ]
        opts = client.get_flight_info.call_args.args[1]
        assert isinstance(opts, flight.FlightCallOptions)
        assert client.do_get.call_args.args[1] is opts


class TestGetTables:
    def test_returns_table_and_sends_command(self, flight_client: FlightClient) -> None:
        table = pa.table({"table_name": ["cities"]})
        client = _fake_client(table)
        with _patched(client):
            result = flight_client.get_tables("my-conn", table_name_filter="cit%", include_schema=True)
        assert result is table
        type_url, fields = _sent_command(client)
        assert type_url == _GET_TABLES_TYPE_URL
        assert fields == {3: b"cit%", 5: 1}
        client.close.assert_called_once()

    @pytest.mark.parametrize("failure_point", ["get_flight_info", "do_get"])
    def test_auth_error_refreshes_token_and_retries(self, failure_point: str) -> None:
        tokens = iter(["old-tok", "new-tok"])
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=lambda: next(tokens))
        table = pa.table({"table_name": ["cities"]})
        rejected = _fake_client(table)
        getattr(rejected, failure_point).side_effect = flight.FlightUnauthenticatedError("token expired")
        accepted = _fake_client(table)
        with _patched(rejected, accepted) as (_, call_options):
            result = client.get_tables("my-conn")

        assert result is table
        sent_tokens = [dict(c.kwargs["headers"])[b"authorization"] for c in call_options.call_args_list]
        assert sent_tokens == [b"Bearer old-tok", b"Bearer new-tok"]
        rejected.close.assert_called_once()

    def test_server_error_maps_to_connection_error(self, flight_client: FlightClient) -> None:
        client = _fake_client(pa.table({"table_name": ["cities"]}))
        client.get_flight_info.side_effect = flight.FlightInternalError("catalog unavailable")
        with _patched(client), pytest.raises(DCHConnectionError, match="catalog unavailable"):
            flight_client.get_tables("my-conn")


class TestTokenProviderGuard:
    def test_token_and_provider_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="Cannot specify both"):
            FlightClient(url=_URL, token="tok", token_provider=lambda: "fresh")


class TestTokenProvider:
    def test_provider_called_once_and_cached(self) -> None:
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        with _patched(_fake_client(pa.table({"col": [1]})), _fake_client(pa.table({"col": [1]}))) as (_, opts):
            client.read("SELECT 1", "conn-1")
            client.read("SELECT 1", "conn-1")
        assert [_sent_headers(opts, i)[b"authorization"] for i in range(2)] == [b"Bearer token-1"] * 2
        assert calls() == 1

    def test_provider_used_for_server_info(self) -> None:
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=lambda: "fresh-token")
        with _patched(_server_info_client()) as (_, call_options):
            client.server_info()
        assert _sent_headers(call_options)[b"authorization"] == b"Bearer fresh-token"

    @pytest.mark.parametrize("failure_point", ["get_flight_info", "do_get"])
    def test_read_auth_error_refreshes_and_retries(self, failure_point: str) -> None:
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        table = pa.table({"col": [1]})
        rejected = _fake_client(table)
        getattr(rejected, failure_point).side_effect = flight.FlightUnauthenticatedError("token expired")
        with _patched(rejected, _fake_client(table)):
            result = client.read("SELECT 1", "conn-1")
        assert calls() == 2
        assert result is table

    def test_auth_error_detected_from_message(self) -> None:
        # Fallback for errors whose type does not carry the gRPC status.
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        rejected = _fake_client(pa.table({"col": [1]}))
        rejected.get_flight_info.side_effect = flight.FlightUnavailableError("UNAUTHENTICATED: token expired")
        with _patched(rejected, _fake_client(pa.table({"col": [1]}))):
            client.read("SELECT 1", "conn-1")
        assert calls() == 2

    def test_auth_error_after_refresh_raises(self) -> None:
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=lambda: "bad-token")
        first, second = _fake_client(), _fake_client()
        for fake in (first, second):
            fake.get_flight_info.side_effect = flight.FlightUnauthenticatedError("UNAUTHENTICATED: invalid")
        with _patched(first, second), pytest.raises(DCHConnectionError, match="UNAUTHENTICATED"):
            client.read("SELECT 1", "conn-1")

    def test_non_auth_error_not_retried(self) -> None:
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        rejected = _fake_client()
        rejected.get_flight_info.side_effect = flight.FlightUnavailableError("connection refused")
        with _patched(rejected) as (connect, _), pytest.raises(DCHConnectionError, match="connection refused"):
            client.read("SELECT 1", "conn-1")
        assert calls() == 1
        connect.assert_called_once()

    def test_static_token_not_retried(self, flight_client: FlightClient) -> None:
        rejected = _fake_client()
        rejected.get_flight_info.side_effect = flight.FlightUnauthenticatedError("expired")
        with _patched(rejected) as (connect, _), pytest.raises(DCHConnectionError, match="expired"):
            flight_client.read("SELECT 1", "conn-1")
        connect.assert_called_once()

    def test_server_info_auth_error_triggers_refresh(self) -> None:
        provider, calls = _counting_provider()
        client = FlightClient(url=_URL, tenant_id="t1", token_provider=provider)
        rejected = _server_info_client()
        rejected.get_flight_info.side_effect = flight.FlightUnauthenticatedError("token expired")
        with _patched(rejected, _server_info_client()):
            result = client.server_info()
        assert calls() == 2
        assert result["vendor_name"] == "Data Connect Hub"


class TestTimeouts:
    @pytest.mark.parametrize(
        ("operation", "make_client"),
        [
            (lambda c: c.read("SELECT 1", "conn-1"), lambda: _fake_client(pa.table({"col": [1]}))),
            (lambda c: list(c.read_batches("SELECT 1", "conn-1")), lambda: _fake_client(pa.table({"col": [1]}))),
            (lambda c: c.get_tables("conn-1"), lambda: _fake_client(pa.table({"col": [1]}))),
            (lambda c: c.server_info(), _server_info_client),
        ],
        ids=["read", "read_batches", "get_tables", "server_info"],
    )
    def test_timeout_applied_to_every_call(
        self, operation: Callable[[FlightClient], Any], make_client: Callable[[], MagicMock]
    ) -> None:
        client = FlightClient(url=_URL, token="tok", tenant_id="t1", timeout=10.0)
        with _patched(make_client()) as (_, call_options):
            operation(client)
        assert call_options.call_args.kwargs["timeout"] == 10.0

    def test_no_timeout_by_default(self, flight_client: FlightClient) -> None:
        with _patched(_fake_client(pa.table({"col": [1]}))) as (_, call_options):
            flight_client.read("SELECT 1", "conn-1")
        assert call_options.call_args.kwargs["timeout"] is None


class TestTLS:
    @staticmethod
    def _connect_kwargs(client: FlightClient) -> dict[str, Any]:
        with _patched(_fake_client(pa.table({"col": [1]}))) as (connect, _):
            client.read("SELECT 1", "conn-1")
        return dict(connect.call_args.kwargs)

    @staticmethod
    def _cert(tmp_path: Any) -> str:
        cert_file = tmp_path / "ca.pem"
        cert_file.write_text("-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----\n")
        return str(cert_file)

    def test_insecure_disables_server_verification(self) -> None:
        client = FlightClient(url="grpc+tls://localhost:50051", token="tok", tenant_id="t1", insecure=True)
        assert self._connect_kwargs(client) == {"disable_server_verification": True}

    def test_ca_cert_sets_tls_root_certs(self, tmp_path: Any) -> None:
        client = FlightClient(
            url="grpc+tls://localhost:50051", token="tok", tenant_id="t1", ca_cert=self._cert(tmp_path)
        )
        kwargs = self._connect_kwargs(client)
        assert b"-----BEGIN CERTIFICATE-----" in kwargs["tls_root_certs"]
        assert "disable_server_verification" not in kwargs

    def test_ca_cert_file_not_found_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="CA certificate file not found"):
            FlightClient(url="grpc+tls://localhost:50051", token="tok", tenant_id="t1", ca_cert="/nonexistent/ca.pem")

    def test_insecure_overrides_ca_cert(self, tmp_path: Any) -> None:
        client = FlightClient(
            url="grpc+tls://localhost:50051",
            token="tok",
            tenant_id="t1",
            ca_cert=self._cert(tmp_path),
            insecure=True,
        )
        assert self._connect_kwargs(client) == {"disable_server_verification": True}

    def test_no_tls_options_by_default(self, flight_client: FlightClient) -> None:
        assert self._connect_kwargs(flight_client) == {}

    def test_real_pyarrow_accepts_tls_options(self, tmp_path: Any) -> None:
        # Unpatched connect (lazy, no network I/O) so the min-deps job catches
        # pyarrow releases that reject these keyword arguments.
        FlightClient(
            url="grpc+tls://localhost:50051", token="tok", tenant_id="t1", insecure=True
        )._flight_connect().close()


class TestMapError:
    @pytest.mark.parametrize(
        ("raised", "default", "expected"),
        [
            (flight.FlightTimedOutError("t"), DCHQueryError, DCHTimeoutError),
            (flight.FlightUnauthenticatedError("a"), DCHQueryError, DCHConnectionError),
            (flight.FlightUnauthorizedError("p"), DCHQueryError, DCHConnectionError),
            (flight.FlightUnavailableError("u"), DCHQueryError, DCHConnectionError),
            (flight.FlightServerError("s"), DCHQueryError, DCHQueryError),
            (flight.FlightServerError("s"), DCHConnectionError, DCHConnectionError),
            (pa.ArrowInvalid("i"), DCHQueryError, DCHQueryError),
            (RuntimeError("r"), DCHQueryError, DCHQueryError),
        ],
    )
    def test_mapping(self, raised: Exception, default: type[DCHError], expected: type[DCHError]) -> None:
        mapped = _map_error(raised, default)
        assert type(mapped) is expected
        assert str(mapped) == str(raised)

    def test_sdk_errors_pass_through(self) -> None:
        original = DCHConfigError("x")
        assert _map_error(original, DCHQueryError) is original


_GATEWAY_URL = "grpc+tls://dch.apps.example.com:443"


class TestGrpcProxyOptions:
    @pytest.fixture(autouse=True)
    def _clean_proxy_env(self, monkeypatch: pytest.MonkeyPatch) -> None:
        for name in list(os.environ):
            if name.lower().endswith("_proxy") or name == "REQUEST_METHOD":
                monkeypatch.delenv(name)

    def test_no_proxy_env_returns_no_options(self) -> None:
        assert _grpc_proxy_options(_GATEWAY_URL) == []

    @pytest.mark.parametrize("name", ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "grpc_proxy"])
    def test_proxy_env_is_passed_explicitly(self, monkeypatch: pytest.MonkeyPatch, name: str) -> None:
        # gRPC C-core ignores the uppercase forms; the SDK must forward them itself.
        monkeypatch.setenv(name, "http://proxy.example.com:3128")
        assert _grpc_proxy_options(_GATEWAY_URL) == [("grpc.http_proxy", "http://proxy.example.com:3128")]

    def test_precedence_matches_grpc(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("http_proxy", "http://http-proxy:3128")
        monkeypatch.setenv("https_proxy", "http://https-proxy:3128")
        assert _grpc_proxy_options(_GATEWAY_URL) == [("grpc.http_proxy", "http://https-proxy:3128")]
        monkeypatch.setenv("grpc_proxy", "http://grpc-proxy:3128")
        assert _grpc_proxy_options(_GATEWAY_URL) == [("grpc.http_proxy", "http://grpc-proxy:3128")]

    @pytest.mark.skipif(sys.platform == "win32", reason="Windows environment variables are case-insensitive")
    def test_lowercase_wins_over_uppercase(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("HTTPS_PROXY", "http://upper:3128")
        monkeypatch.setenv("https_proxy", "http://lower:3128")
        assert _grpc_proxy_options(_GATEWAY_URL) == [("grpc.http_proxy", "http://lower:3128")]
        monkeypatch.setenv("NO_PROXY", "")
        monkeypatch.setenv("no_proxy", ".example.com")
        assert _grpc_proxy_options(_GATEWAY_URL) == [("grpc.enable_http_proxy", 0)]

    @pytest.mark.parametrize(
        "url",
        [
            "grpc://127.0.0.1:18443",
            "grpc://127.1.2.3:18443",
            "grpc://localhost:18443",
            "grpc://LocalHost:18443",
            "grpc://dch.localhost:18443",
            "grpc://[::1]:18443",
            "grpc://0.0.0.0:18443",
        ],
    )
    def test_loopback_bypasses_proxy_without_no_proxy(self, monkeypatch: pytest.MonkeyPatch, url: str) -> None:
        # e.g. oc port-forward: loopback targets are never proxied.
        monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example.com:3128")
        assert _grpc_proxy_options(url) == [("grpc.enable_http_proxy", 0)]

    def test_scheme_less_proxy_gets_http_scheme(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("HTTPS_PROXY", "proxy.example.com:3128")
        assert _grpc_proxy_options(_GATEWAY_URL) == [("grpc.http_proxy", "http://proxy.example.com:3128")]

    @pytest.mark.parametrize(
        ("url", "no_proxy"),
        [
            (_GATEWAY_URL, ".example.com"),
            (_GATEWAY_URL, "localhost,dch.apps.example.com"),
            (_GATEWAY_URL, "*"),
            ("grpc://10.1.2.3:50051", "localhost,10.0.0.0/8"),
            ("grpc://[fd00::1]:50051", "fd00::/8"),
            (_GATEWAY_URL, "dch.apps.example.com:443"),
            (_GATEWAY_URL, ".example.com:443"),
            (_GATEWAY_URL, "*.example.com"),
            # Portless URL (as built by _build_urls for a bare host): gRPC dials 443.
            ("grpc+tls://dch.apps.example.com", "dch.apps.example.com:443"),
            ("grpc://10.1.2.3", "10.1.2.3:443"),
            ("grpc://10.1.2.3:50051", "10.1.2.3:50051"),
            ("grpc://[fd00::1]:50051", "[fd00::1]:50051"),
        ],
    )
    def test_no_proxy_match_disables_proxy(self, monkeypatch: pytest.MonkeyPatch, url: str, no_proxy: str) -> None:
        monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example.com:3128")
        monkeypatch.setenv("NO_PROXY", no_proxy)
        assert _grpc_proxy_options(url) == [("grpc.enable_http_proxy", 0)]

    @pytest.mark.parametrize(
        ("url", "no_proxy"),
        [
            (_GATEWAY_URL, ".svc,.cluster.local,10.0.0.0/8"),
            ("grpc+tls://dch.notexample.com:443", "example.com"),
            ("grpc+tls://dch.notexample.com:443", "*.example.com"),
            ("grpc+tls://dch.apps.example.com", "dch.apps.example.com:8443"),
            ("grpc://10.1.2.3", "10.1.2.3:80"),
            ("grpc://192.168.1.1:50051", "10.0.0.0/8,localhost"),
            (_GATEWAY_URL, "dch.apps.example.com:8443"),
            ("grpc://10.1.2.3:50051", "10.1.2.3:443"),
        ],
    )
    def test_no_proxy_without_match_keeps_proxy(self, monkeypatch: pytest.MonkeyPatch, url: str, no_proxy: str) -> None:
        monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example.com:3128")
        monkeypatch.setenv("NO_PROXY", no_proxy)
        assert _grpc_proxy_options(url) == [("grpc.http_proxy", "http://proxy.example.com:3128")]

    def test_build_urls_bare_host_matches_port_qualified_no_proxy(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example.com:3128")
        monkeypatch.setenv("NO_PROXY", "dch.apps.example.com:443")
        _, flight_url = _build_urls("dch.apps.example.com")
        assert flight_url == "grpc+tls://dch.apps.example.com"
        assert _grpc_proxy_options(flight_url) == [("grpc.enable_http_proxy", 0)]

    def test_unix_socket_url_returns_no_options(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example.com:3128")
        assert _grpc_proxy_options("grpc+unix:///tmp/flight.sock") == []

    def test_flight_connect_forwards_proxy_options(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example.com:3128")
        client = FlightClient(url=_GATEWAY_URL, token="tok", tenant_id="t1")
        with patch("data_connect_hub._flight.flight.connect") as connect:
            client._flight_connect()
        assert connect.call_args.kwargs["generic_options"] == [("grpc.http_proxy", "http://proxy.example.com:3128")]

    def test_flight_connect_omits_generic_options_without_proxy(self) -> None:
        client = FlightClient(url=_GATEWAY_URL, token="tok", tenant_id="t1")
        with patch("data_connect_hub._flight.flight.connect") as connect:
            client._flight_connect()
        assert "generic_options" not in connect.call_args.kwargs

    def test_real_pyarrow_accepts_proxy_options(self, monkeypatch: pytest.MonkeyPatch) -> None:
        # Unpatched connect (lazy, no network I/O) so the min-deps job catches
        # pyarrow releases that reject these channel args.
        monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example.com:3128")
        FlightClient(url=_GATEWAY_URL, token="tok", tenant_id="t1")._flight_connect().close()
        monkeypatch.setenv("NO_PROXY", ".example.com")
        FlightClient(url=_GATEWAY_URL, token="tok", tenant_id="t1")._flight_connect().close()
