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
from adbc_driver_flightsql import DatabaseOptions

from ._auth import ADBC_HEADER_PREFIX, TokenCache, build_flight_headers
from .exceptions import DCHConfigError, DCHConnectionError, DCHQueryError

_GRPC_UNAUTHENTICATED = "unauthenticated"

_QUERY_TIMEOUT = DatabaseOptions.TIMEOUT_QUERY.value
_FETCH_TIMEOUT = DatabaseOptions.TIMEOUT_FETCH.value
_TLS_ROOT_CERTS = DatabaseOptions.TLS_ROOT_CERTS.value
_TLS_SKIP_VERIFY = DatabaseOptions.TLS_SKIP_VERIFY.value


def _is_auth_error(exc: Exception) -> bool:
    return _GRPC_UNAUTHENTICATED in str(exc).lower()


class FlightSQLClient:
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
        self._static_kwargs: dict[str, str] = {}
        if timeout is not None:
            self._static_kwargs[_QUERY_TIMEOUT] = str(timeout)
            self._static_kwargs[_FETCH_TIMEOUT] = str(timeout)
        if insecure:
            self._static_kwargs[_TLS_SKIP_VERIFY] = "true"
        elif ca_cert:
            cert_path = Path(ca_cert)
            if not cert_path.is_file():
                raise DCHConfigError(f"CA certificate file not found: {ca_cert}")
            self._static_kwargs[_TLS_ROOT_CERTS] = cert_path.read_text()
        if not token_provider:
            self._static_kwargs.update(build_flight_headers(token=token, tenant_id=tenant_id))

    def _base_kwargs(self) -> dict[str, str]:
        if self._token_cache:
            headers = build_flight_headers(token=self._token_cache.get(), tenant_id=self._tenant_id)
            return {**self._static_kwargs, **headers}
        return dict(self._static_kwargs)

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
            return {str(k): v for k, v in conn.adbc_get_info().items()}
        except flight_dbapi.Error as exc:
            raise DCHConnectionError(str(exc)) from exc
        finally:
            conn.close()

    def close(self) -> None:
        """No-op — connections are opened and closed per call."""
