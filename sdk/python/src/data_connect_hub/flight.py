"""Arrow Flight SQL client for tabular data queries."""

from __future__ import annotations

import contextlib
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    import pandas as pd

import adbc_driver_flightsql.dbapi as flight_dbapi
import pyarrow as pa
import pyarrow.flight as flight
from adbc_driver_flightsql import DatabaseOptions

from ._auth import ADBC_HEADER_PREFIX, TokenCache, build_headers
from .exceptions import DCHConfigError, DCHConnectionError, DCHQueryError

_GRPC_UNAUTHENTICATED = "unauthenticated"
_CMD_GET_TABLES_TYPE_URL = "type.googleapis.com/arrow.flight.protocol.sql.CommandGetTables"

_QUERY_TIMEOUT = DatabaseOptions.TIMEOUT_QUERY.value
_FETCH_TIMEOUT = DatabaseOptions.TIMEOUT_FETCH.value
_TLS_ROOT_CERTS = DatabaseOptions.TLS_ROOT_CERTS.value
_TLS_SKIP_VERIFY = DatabaseOptions.TLS_SKIP_VERIFY.value

_FLIGHT_DISABLE_SERVER_VERIFICATION = "disable_server_verification"
_FLIGHT_TLS_ROOT_CERTS = "tls_root_certs"


def _is_auth_error(exc: Exception) -> bool:
    return _GRPC_UNAUTHENTICATED in str(exc).lower()


class FlightClient:
    """Thin wrapper around ADBC Flight SQL for Data Connect Hub queries.

    Parameters
    ----------
    flight_url : str
        gRPC endpoint, e.g. ``grpc://host:50051`` or ``grpc+tls://host:50051``.
    token : str
        Static Bearer token value.
    tenant_id : str
        Tenant identifier.
    token_provider : Callable[[], str], optional
        A callable that returns a valid Bearer token string.  The SDK calls
        this once and caches the result.  On authentication failure, the
        token is refreshed automatically.  Mutually exclusive with *token*.
    timeout : float, optional
        Timeout in seconds for Flight SQL RPC calls (applies to both query and fetch).
    ca_cert : str, optional
        Path to a PEM-encoded CA certificate file for TLS verification.
    insecure : bool
        Skip TLS certificate verification (default False).
    """

    def __init__(
        self,
        flight_url: str,
        token: str = "",
        tenant_id: str = "",
        *,
        token_provider: Callable[[], str] | None = None,
        timeout: float | None = None,
        ca_cert: str | None = None,
        insecure: bool = False,
    ) -> None:
        if token and token_provider:
            raise DCHConfigError(
                "Cannot specify both 'token' and 'token_provider'."
                " Please provide either a static token or a token_provider callable, not both."
            )
        self._flight_url = flight_url
        self._tenant_id = tenant_id
        self._token_cache: TokenCache | None = TokenCache(token_provider) if token_provider else None
        self._insecure = insecure
        self._tls_root_certs: str | None = None
        self._adbc_opts: dict[str, str] = {}
        if timeout is not None:
            self._adbc_opts[_QUERY_TIMEOUT] = str(timeout)
            self._adbc_opts[_FETCH_TIMEOUT] = str(timeout)
        if insecure:
            self._adbc_opts[_TLS_SKIP_VERIFY] = "true"
        elif ca_cert:
            cert_path = Path(ca_cert)
            if not cert_path.is_file():
                raise DCHConfigError(f"CA certificate file not found: {ca_cert}")
            self._tls_root_certs = cert_path.read_text()
            self._adbc_opts[_TLS_ROOT_CERTS] = self._tls_root_certs
        self._static_headers: dict[str, str] = {}
        if not token_provider:
            self._static_headers = build_headers(token=token, tenant_id=tenant_id)

    def _headers(self) -> dict[str, str]:
        if self._token_cache:
            return build_headers(token=self._token_cache.get(), tenant_id=self._tenant_id)
        return dict(self._static_headers)

    def _base_kwargs(self) -> dict[str, str]:
        prefixed = {f"{ADBC_HEADER_PREFIX}{k.lower()}": v for k, v in self._headers().items()}
        return {**self._adbc_opts, **prefixed}

    def _call_options(self) -> flight.FlightCallOptions:
        return flight.FlightCallOptions(headers=[(k.lower().encode(), v.encode()) for k, v in self._headers().items()])

    def _flight_connect(self) -> flight.FlightClient:
        kwargs: dict[str, Any] = {}
        if self._insecure:
            kwargs[_FLIGHT_DISABLE_SERVER_VERIFICATION] = True
        if self._tls_root_certs:
            kwargs[_FLIGHT_TLS_ROOT_CERTS] = self._tls_root_certs.encode()
        try:
            return flight.connect(self._flight_url, **kwargs)
        except Exception as exc:
            raise DCHConnectionError(str(exc)) from exc

    def _connect(self, connection_id: str) -> flight_dbapi.Connection:
        db_kwargs = {
            **self._base_kwargs(),
            f"{ADBC_HEADER_PREFIX}x-data-connection-id": connection_id,
        }
        try:
            return flight_dbapi.connect(self._flight_url, db_kwargs=db_kwargs)
        except flight_dbapi.Error as exc:
            raise DCHConnectionError(str(exc)) from exc

    def read(self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None) -> pa.Table:
        """Execute *sql* and return the full result as a PyArrow Table."""
        try:
            return self._do_read(sql, connection_id, parameters=parameters)
        except DCHConnectionError as exc:
            if self._token_cache is not None and _is_auth_error(exc):
                self._token_cache.refresh()
                return self._do_read(sql, connection_id, parameters=parameters)
            raise

    def _do_read(self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None) -> pa.Table:
        conn = self._connect(connection_id)
        try:
            cursor = conn.cursor()
            try:
                cursor.execute(sql, parameters)
                return cursor.fetch_arrow_table()
            except flight_dbapi.Error as exc:
                raise DCHQueryError(str(exc)) from exc
            finally:
                with contextlib.suppress(Exception):
                    cursor.close()
        finally:
            conn.close()

    def read_pandas(self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None) -> pd.DataFrame:
        """Execute *sql* and return the result as a pandas DataFrame."""
        return self.read(sql, connection_id, parameters=parameters).to_pandas()

    def server_info(self) -> dict[str, Any]:
        """Return Flight SQL server metadata."""
        try:
            return self._do_server_info()
        except DCHConnectionError as exc:
            if self._token_cache is not None and _is_auth_error(exc):
                self._token_cache.refresh()
                return self._do_server_info()
            raise

    def _do_server_info(self) -> dict[str, Any]:
        try:
            conn = flight_dbapi.connect(self._flight_url, db_kwargs=self._base_kwargs())
        except flight_dbapi.Error as exc:
            raise DCHConnectionError(str(exc)) from exc
        try:
            info: dict[str, Any] = {str(k): v for k, v in conn.adbc_get_info().items()}
            info["supported_connectors"] = self._do_supported_connectors()
            return info
        except flight_dbapi.Error as exc:
            raise DCHConnectionError(str(exc)) from exc
        finally:
            conn.close()

    def _do_supported_connectors(self) -> list[str]:
        client = self._flight_connect()
        try:
            action = flight.Action("GetSupportedConnectors", b"")
            results = list(client.do_action(action, self._call_options()))
            if not results:
                return []
            body = results[0].body.to_pybytes()
            reader = pa.ipc.open_stream(body)
            table = reader.read_all()
            connectors: list[str] = table.column("name").to_pylist()
            return connectors
        except Exception as exc:
            raise DCHConnectionError(str(exc)) from exc
        finally:
            client.close()

    def get_tables(
        self,
        connection_id: str,
        *,
        table_name_filter: str | None = None,
        include_schema: bool = False,
    ) -> pa.Table:
        """Retrieve table metadata via Flight SQL ``CommandGetTables``.

        Parameters
        ----------
        connection_id : str
            Data-connection ID to query.
        table_name_filter : str, optional
            SQL ``LIKE`` pattern to filter table names (e.g. ``"cities"``).
        include_schema : bool
            If True, the response includes each table's Arrow schema
            serialized as IPC bytes in a ``table_schema`` column.
        """
        try:
            return self._do_get_tables(
                connection_id, table_name_filter=table_name_filter, include_schema=include_schema
            )
        except DCHConnectionError as exc:
            if self._token_cache is not None and _is_auth_error(exc):
                self._token_cache.refresh()
                return self._do_get_tables(
                    connection_id, table_name_filter=table_name_filter, include_schema=include_schema
                )
            raise

    def _do_get_tables(
        self,
        connection_id: str,
        *,
        table_name_filter: str | None = None,
        include_schema: bool = False,
    ) -> pa.Table:
        cmd = _build_command_get_tables(
            table_name_filter_pattern=table_name_filter,
            include_schema=include_schema,
        )

        headers = [
            *self._call_options().headers,
            (b"x-data-connection-id", connection_id.encode()),
        ]
        opts = flight.FlightCallOptions(headers=headers)

        client = self._flight_connect()
        try:
            descriptor = flight.FlightDescriptor.for_command(cmd)
            info = client.get_flight_info(descriptor, opts)
            if not info.endpoints:
                raise DCHConnectionError("server returned no Flight endpoints for CommandGetTables")
            reader = client.do_get(info.endpoints[0].ticket, opts)
            return reader.read_all()
        except Exception as exc:
            raise DCHConnectionError(str(exc)) from exc
        finally:
            client.close()

    def close(self) -> None:
        """No-op — connections are opened and closed per call."""


def _encode_varint(value: int) -> bytes:
    result = bytearray()
    while value > 0x7F:
        result.append((value & 0x7F) | 0x80)
        value >>= 7
    result.append(value & 0x7F)
    return bytes(result)


# CommandGetTables protobuf field numbers (from FlightSql.proto)
_CGT_FIELD_TABLE_NAME_FILTER = 3
_CGT_FIELD_INCLUDE_SCHEMA = 5


def _build_command_get_tables(
    *,
    table_name_filter_pattern: str | None = None,
    include_schema: bool = False,
) -> bytes:
    """Serialize a Flight SQL ``CommandGetTables`` wrapped in ``google.protobuf.Any``."""
    from google.protobuf import any_pb2  # type: ignore[import-untyped]

    msg = b""
    if table_name_filter_pattern is not None:
        encoded = table_name_filter_pattern.encode("utf-8")
        msg += _encode_varint((_CGT_FIELD_TABLE_NAME_FILTER << 3) | 2) + _encode_varint(len(encoded)) + encoded
    if include_schema:
        msg += _encode_varint((_CGT_FIELD_INCLUDE_SCHEMA << 3) | 0) + _encode_varint(1)

    any_msg = any_pb2.Any()
    any_msg.type_url = _CMD_GET_TABLES_TYPE_URL
    any_msg.value = msg
    return bytes(any_msg.SerializeToString())
