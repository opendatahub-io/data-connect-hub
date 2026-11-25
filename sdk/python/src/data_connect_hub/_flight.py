"""Arrow Flight SQL client for tabular data queries.

All Flight traffic goes through ``pyarrow.flight``, so the SDK has a single
Flight TLS stack. With a pyarrow build that links the system OpenSSL (for
example from the Red Hat AI Python index on RHEL/UBI), Flight TLS runs in the
host's cryptographic module, the same one REST uses through Python's ``ssl``.

Flight SQL commands are small protobuf messages, encoded by hand in
``_flight_sql_proto`` so the ``flight`` extra does not depend on ``protobuf``.
"""

from __future__ import annotations

import contextlib
import ipaddress
import json
import urllib.parse
import urllib.request
from collections.abc import Callable, Generator, Iterator
from pathlib import Path
from typing import TYPE_CHECKING, Any, TypeVar

if TYPE_CHECKING:
    import pandas as pd

import pyarrow as pa
import pyarrow.flight as flight

from ._auth import TokenCache, build_headers
from ._flight_sql_proto import (
    build_command_get_sql_info,
    build_command_get_tables,
    build_command_statement_query,
)
from .exceptions import DCHConfigError, DCHConnectionError, DCHError, DCHQueryError, DCHTimeoutError

_T = TypeVar("_T")

_GRPC_UNAUTHENTICATED = "unauthenticated"

# Flight SQL SqlInfo ids (FlightSql.proto) and the server_info() keys they map to.
_SQL_INFO_KEYS: dict[int, str] = {
    0: "vendor_name",  # FLIGHT_SQL_SERVER_NAME
    1: "vendor_version",  # FLIGHT_SQL_SERVER_VERSION
    2: "vendor_arrow_version",  # FLIGHT_SQL_SERVER_ARROW_VERSION
}
_DRIVER_NAME = "Data Connect Hub Python SDK (pyarrow.flight)"

# Endpoint location that tells the client to reuse the current connection (Flight spec).
_REUSE_CONNECTION_URI = "arrow-flight-reuse-connection://?"

_ACTION_GET_SUPPORTED_CONNECTORS = "GetSupportedConnectors"
_HEADER_CONNECTION_ID = "x-data-connection-id"

_FLIGHT_DISABLE_SERVER_VERIFICATION = "disable_server_verification"
_FLIGHT_TLS_ROOT_CERTS = "tls_root_certs"
_FLIGHT_GENERIC_OPTIONS = "generic_options"

_GRPC_ARG_HTTP_PROXY = "grpc.http_proxy"
_GRPC_ARG_ENABLE_HTTP_PROXY = "grpc.enable_http_proxy"

# gRPC statuses meaning the call never reached query execution: the server
# rejected the caller or could not be reached.
_CONNECTION_ERRORS: tuple[type[Exception], ...] = (
    flight.FlightUnauthenticatedError,
    flight.FlightUnauthorizedError,
    flight.FlightUnavailableError,
)


def _is_auth_error(exc: BaseException) -> bool:
    if isinstance(exc, flight.FlightUnauthenticatedError) or isinstance(
        exc.__cause__, flight.FlightUnauthenticatedError
    ):
        return True
    return _GRPC_UNAUTHENTICATED in str(exc).lower()


def _map_error(exc: Exception, default: type[DCHError]) -> DCHError:
    """Translate a pyarrow/gRPC exception into the SDK exception hierarchy.

    Authentication, authorization and availability failures become
    :class:`DCHConnectionError`, an expired deadline becomes
    :class:`DCHTimeoutError`, and anything else becomes *default*
    (:class:`DCHQueryError` for queries, :class:`DCHConnectionError` for
    metadata calls).
    """
    if isinstance(exc, DCHError):
        return exc
    if isinstance(exc, flight.FlightTimedOutError):
        return DCHTimeoutError(str(exc))
    if isinstance(exc, _CONNECTION_ERRORS):
        return DCHConnectionError(str(exc))
    return default(str(exc))


def _split_host_port(entry: str) -> tuple[str, int | None]:
    """Split a ``no_proxy`` entry into host and optional port (``host:443``, ``[::1]:443``)."""
    if entry.startswith("["):
        host, _, rest = entry[1:].partition("]")
        port = rest[1:] if rest.startswith(":") else ""
    elif entry.count(":") == 1:
        host, _, port = entry.partition(":")
    else:
        return entry, None
    return host, int(port) if port.isdigit() else None


def _bypasses_proxy(host: str, port: int | None, no_proxy: str) -> bool:
    """Return True when ``host:port`` must be reached directly.

    Loopback targets (``localhost``, ``*.localhost``, loopback/unspecified IPs)
    always bypass the proxy, so ``oc port-forward`` works behind a corporate
    proxy. Otherwise ``no_proxy``
    entries match as ``*``, domain suffix (``example.com``, ``.example.com`` or
    ``*.example.com``) or IP/CIDR, optionally restricted to a port (``host:443``).
    """
    host = host.lower()
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        ip = None
    if host == "localhost" or host.endswith(".localhost") or (ip is not None and (ip.is_loopback or ip.is_unspecified)):
        return True
    for raw in no_proxy.split(","):
        entry = raw.strip().lower()
        if entry == "*":
            return True
        entry_host, entry_port = _split_host_port(entry)
        if not entry_host or (entry_port is not None and entry_port != port):
            continue
        if ip is not None:
            with contextlib.suppress(ValueError):
                if ip in ipaddress.ip_network(entry_host, strict=False):
                    return True
            continue
        domain = entry_host.removeprefix("*.").lstrip(".")
        if host == domain or host.endswith(f".{domain}"):
            return True
    return False


def _grpc_proxy_options(url: str) -> list[tuple[str, str | int]]:
    """Return gRPC channel args that apply the proxy environment to a pyarrow Flight client.

    gRPC C-core (used by pyarrow Flight) only reads the lowercase ``grpc_proxy``,
    ``https_proxy``, ``http_proxy`` and ``no_proxy`` variables, while httpx (REST)
    also honors the uppercase forms. Resolving the proxy here, case-insensitively,
    makes Flight honor the same variables as REST. Loopback targets are never
    proxied; httpx proxies them unless they are listed in ``no_proxy``.
    """
    parts = urllib.parse.urlsplit(url)
    if not parts.hostname:
        return []
    proxies = urllib.request.getproxies_environment()
    proxy = proxies.get("grpc") or proxies.get("https") or proxies.get("http")
    if not proxy:
        return []
    # gRPC dials 443 when the Flight URL omits the port, for every scheme
    # (grpc://, grpc+tcp:// and grpc+tls://), so match no_proxy against that.
    if _bypasses_proxy(parts.hostname, parts.port or 443, proxies.get("no", "")):
        return [(_GRPC_ARG_ENABLE_HTTP_PROXY, 0)]
    if "://" not in proxy:
        proxy = f"http://{proxy}"
    return [(_GRPC_ARG_HTTP_PROXY, proxy)]


class _UnsupportedEndpointError(Exception):
    """The server pointed a Flight endpoint at another location."""


def _check_endpoint(endpoint: Any) -> None:
    """Reject endpoints that would require dialing another server.

    The DCH Flight service returns endpoints without locations, which means
    "fetch from the server that answered ``GetFlightInfo``".
    """
    for location in endpoint.locations:
        uri = location.uri
        uri = uri.decode() if isinstance(uri, bytes) else str(uri)
        if uri != _REUSE_CONNECTION_URI:
            raise _UnsupportedEndpointError(
                f"server returned a Flight endpoint at another location ({uri}), which the SDK does not support"
            )


def _read_all(client: Any, command: bytes, opts: Any) -> pa.Table:
    """Run ``GetFlightInfo`` for *command* and ``DoGet`` every endpoint into one table."""
    info = client.get_flight_info(flight.FlightDescriptor.for_command(command), opts)
    tables = []
    for endpoint in info.endpoints:
        _check_endpoint(endpoint)
        tables.append(client.do_get(endpoint.ticket, opts).read_all())
    if not tables:
        return info.schema.empty_table()
    return tables[0] if len(tables) == 1 else pa.concat_tables(tables)


def _iter_reader(reader: Any) -> Generator[pa.RecordBatch, None, None]:
    """Yield the record batches of a ``FlightStreamReader``, cancelling the stream if abandoned early."""
    exhausted = False
    try:
        while True:
            try:
                chunk = reader.read_chunk()
            except StopIteration:
                break
            if chunk.data is not None:
                yield chunk.data
        exhausted = True
    finally:
        if not exhausted:
            with contextlib.suppress(Exception):
                reader.cancel()


def _supported_connectors(client: Any, opts: Any) -> list[str]:
    action = flight.Action(_ACTION_GET_SUPPORTED_CONNECTORS, b"")
    results = list(client.do_action(action, opts))
    if not results:
        return []
    body = results[0].body.to_pybytes()
    try:
        table = pa.ipc.open_stream(body).read_all()
        connectors: list[str] = table.column("name").to_pylist()
    except pa.ArrowInvalid:
        connectors = [str(entry["name"]) for entry in json.loads(body)]
    return connectors


def _sdk_version() -> str:
    from . import __version__

    return __version__


class FlightClient:
    """Flight SQL client for Data Connect Hub queries, built on ``pyarrow.flight``.

    Every call opens a Flight connection, sends the authentication, tenant and
    (for data calls) connection-ID headers, and closes the connection when done.

    Parameters
    ----------
    url : str
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
        Deadline in seconds for each Flight RPC.  For ``DoGet`` the deadline
        covers the whole result stream.
    ca_cert : str, optional
        Path to a PEM-encoded CA certificate file for TLS verification.
    insecure : bool
        Skip TLS certificate verification (default False).
    """

    def __init__(
        self,
        url: str,
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
        self._url = url
        self._tenant_id = tenant_id
        self._timeout = timeout
        self._token_cache: TokenCache | None = TokenCache(token_provider) if token_provider else None
        self._insecure = insecure
        self._tls_root_certs: str | None = None
        if not insecure and ca_cert:
            cert_path = Path(ca_cert)
            if not cert_path.is_file():
                raise DCHConfigError(f"CA certificate file not found: {ca_cert}")
            self._tls_root_certs = cert_path.read_text()
        self._static_headers: dict[str, str] = {}
        if not token_provider:
            self._static_headers = build_headers(token=token, tenant_id=tenant_id)

    # -- connection plumbing --

    def _headers(self) -> dict[str, str]:
        if not self._tenant_id:
            raise DCHConfigError("tenant_id must be provided for Flight operations")
        if self._token_cache:
            return build_headers(token=self._token_cache.get(), tenant_id=self._tenant_id)
        return dict(self._static_headers)

    def _flight_headers(self, connection_id: str | None = None) -> list[tuple[bytes, bytes]]:
        # Callers that need extra headers must extend this list instead of reading
        # FlightCallOptions.headers, which only exists in pyarrow >= 22.
        headers = [(k.lower().encode(), v.encode()) for k, v in self._headers().items()]
        if connection_id is not None:
            headers.append((_HEADER_CONNECTION_ID.encode(), connection_id.encode()))
        return headers

    def _call_options(self, connection_id: str | None = None) -> flight.FlightCallOptions:
        return flight.FlightCallOptions(headers=self._flight_headers(connection_id), timeout=self._timeout)

    def _flight_connect(self) -> flight.FlightClient:
        kwargs: dict[str, Any] = {}
        if self._insecure:
            kwargs[_FLIGHT_DISABLE_SERVER_VERIFICATION] = True
        if self._tls_root_certs:
            kwargs[_FLIGHT_TLS_ROOT_CERTS] = self._tls_root_certs.encode()
        proxy_options = _grpc_proxy_options(self._url)
        if proxy_options:
            kwargs[_FLIGHT_GENERIC_OPTIONS] = proxy_options
        try:
            return flight.connect(self._url, **kwargs)
        except Exception as exc:
            raise DCHConnectionError(str(exc)) from exc

    @contextlib.contextmanager
    def _session(self, default: type[DCHError]) -> Iterator[flight.FlightClient]:
        """Open a Flight connection, map pyarrow errors with *default* and always close it."""
        client = self._flight_connect()
        try:
            yield client
        except DCHError:
            raise
        except Exception as exc:
            raise _map_error(exc, default) from exc
        finally:
            with contextlib.suppress(Exception):
                client.close()

    def _with_auth_retry(self, operation: Callable[[], _T]) -> _T:
        """Run *operation*, refreshing the token and retrying once if the server rejects it."""
        try:
            return operation()
        except DCHConnectionError as exc:
            if self._token_cache is not None and _is_auth_error(exc):
                self._token_cache.refresh()
                return operation()
            raise

    def _fetch(self, command: bytes, connection_id: str, default: type[DCHError]) -> pa.Table:
        opts = self._call_options(connection_id)
        with self._session(default) as client:
            return _read_all(client, command, opts)

    # -- queries --

    def read(self, sql: str, connection_id: str) -> pa.Table:
        """Execute *sql* and return the full result as a PyArrow Table."""
        command = build_command_statement_query(sql)
        return self._with_auth_retry(lambda: self._fetch(command, connection_id, DCHQueryError))

    def read_batches(self, sql: str, connection_id: str) -> Generator[pa.RecordBatch, None, None]:
        """Execute *sql* and return a streaming iterator of RecordBatches.

        Yields one :class:`pyarrow.RecordBatch` per iteration.  The
        connection is opened when the generator is first iterated and closed
        when it is exhausted or closed; closing it early cancels the stream::

            for batch in client.read_batches("SELECT ...", "conn-1"):
                process(batch)
        """
        return self._iter_batches(sql, connection_id)

    def _iter_batches(self, sql: str, connection_id: str) -> Generator[pa.RecordBatch, None, None]:
        command = build_command_statement_query(sql)
        retried = False
        yielded = False
        while True:
            opts = self._call_options(connection_id)
            client = self._flight_connect()
            try:
                info = client.get_flight_info(flight.FlightDescriptor.for_command(command), opts)
                for endpoint in info.endpoints:
                    _check_endpoint(endpoint)
                    for batch in _iter_reader(client.do_get(endpoint.ticket, opts)):
                        yielded = True
                        yield batch
                return
            except DCHError:
                raise
            except Exception as exc:
                mapped = _map_error(exc, DCHQueryError)
                # Retrying is only safe before the caller has seen any rows.
                if not yielded and not retried and self._token_cache is not None and _is_auth_error(exc):
                    self._token_cache.refresh()
                    retried = True
                    continue
                raise mapped from exc
            finally:
                with contextlib.suppress(Exception):
                    client.close()

    def read_pandas(self, sql: str, connection_id: str) -> pd.DataFrame:
        """Execute *sql* and return the result as a pandas DataFrame."""
        return self.read(sql, connection_id).to_pandas()

    # -- metadata --

    def server_info(self) -> dict[str, Any]:
        """Return Flight SQL server metadata.

        Keys: ``vendor_name``, ``vendor_version`` and ``vendor_arrow_version``
        (from Flight SQL ``GetSqlInfo``), ``driver_name``, ``driver_version`` and
        ``driver_arrow_version`` (this SDK and its pyarrow), and
        ``supported_connectors`` (connector types the Flight service can run).
        """
        return self._with_auth_retry(self._do_server_info)

    def _do_server_info(self) -> dict[str, Any]:
        opts = self._call_options()
        with self._session(DCHConnectionError) as client:
            table = _read_all(client, build_command_get_sql_info(list(_SQL_INFO_KEYS)), opts)
            info: dict[str, Any] = {}
            for code, value in zip(
                table.column("info_name").to_pylist(), table.column("value").to_pylist(), strict=True
            ):
                key = _SQL_INFO_KEYS.get(code)
                if key is not None:
                    info[key] = value
            info["driver_name"] = _DRIVER_NAME
            info["driver_version"] = _sdk_version()
            info["driver_arrow_version"] = pa.__version__
            info["supported_connectors"] = _supported_connectors(client, opts)
            return info

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
        command = build_command_get_tables(
            table_name_filter_pattern=table_name_filter,
            include_schema=include_schema,
        )
        return self._with_auth_retry(lambda: self._fetch(command, connection_id, DCHConnectionError))

    def close(self) -> None:
        """No-op — connections are opened and closed per call."""
