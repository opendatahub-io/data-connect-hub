"""Unified DataConnectClient for REST and Flight SQL access."""

from __future__ import annotations

from collections.abc import Callable, Sequence
from typing import TYPE_CHECKING, Any
from urllib.parse import urlparse

from ._rest import RestClient
from .exceptions import DCHConfigError
from .models import (
    ConnectionType,
    CreateConnectionRequest,
    CreateConnectionTypeRequest,
    CredentialField,
    CredentialsRef,
    DataConnection,
    DataFormat,
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)

if TYPE_CHECKING:
    from collections.abc import Generator

    import pandas as pd
    import pyarrow as pa

    from ._flight import FlightClient


def _build_urls(endpoint: str) -> tuple[str, str]:
    """Derive REST and Flight SQL URLs from a single gateway *endpoint*.

    Accepts a bare host, ``host:port``, or a URL carrying a scheme; any scheme
    given is discarded.  Only TLS is supported (``https`` and ``grpc+tls``);
    the ``insecure`` flag on the client controls certificate verification, not
    the transport.

    Returns ``(rest_url, flight_url)``.
    """
    endpoint = endpoint.strip().rstrip("/")
    if not endpoint:
        raise DCHConfigError("endpoint must not be empty")

    parsed = urlparse(endpoint if "://" in endpoint else f"//{endpoint}", scheme="https")

    if parsed.username or parsed.password:
        raise DCHConfigError("endpoint must not contain credentials; pass a token or token_provider instead")

    try:
        hostname, port = parsed.hostname, parsed.port
    except ValueError as exc:
        raise DCHConfigError(f"invalid port in endpoint {endpoint!r}: {exc}") from exc

    # ``urlparse`` reports an explicit ``:0`` as port 0, which would otherwise
    # be indistinguishable from "no port" and silently fall back to 443.
    if port == 0:
        raise DCHConfigError(f"invalid port in endpoint {endpoint!r}: port must be between 1 and 65535")

    if not hostname:
        raise DCHConfigError(f"unable to extract host from endpoint: {endpoint!r}")
    if parsed.path:
        raise DCHConfigError(f"endpoint must not contain a path, got {parsed.path!r} in {endpoint!r}")
    if parsed.query or parsed.fragment:
        raise DCHConfigError(f"endpoint must not contain a query string or fragment: {endpoint!r}")

    # IPv6 literals lose their brackets via ``hostname``; restore them.
    netloc = f"[{hostname}]" if ":" in hostname else hostname
    if port is not None:
        netloc = f"{netloc}:{port}"

    return f"https://{netloc}", f"grpc+tls://{netloc}"


class DataConnectClient:
    """Single entry point for all DCH interactions.

    Parameters
    ----------
    endpoint : str
        Gateway host or host:port (e.g. ``gateway.example.com:8443``).  A
        scheme is not required and is ignored if present.  The SDK derives
        the HTTPS and gRPC+TLS URLs automatically.  Only TLS endpoints are
        supported.
    token : str
        Static Bearer token value (without the "Bearer " prefix).
    token_provider : Callable[[], str], optional
        A callable that returns a valid Bearer token string.  The SDK calls
        this once and caches the result.  If a request receives a 401
        Unauthorized response, the SDK automatically refreshes the token
        by calling the provider again and retries the request.  Mutually
        exclusive with *token*.
    tenant_id : str
        Tenant identifier sent via ``x-tenant-id`` header.
    api_base : str
        API path prefix (default ``/api/v1/data``).
    rest_timeout : float
        HTTP request timeout in seconds (default 30.0).
    ca_cert : str, optional
        Path to a CA certificate file for TLS verification.
    insecure : bool
        Skip TLS certificate verification (default False).
    max_retries : int
        Maximum retry attempts for transient errors (default 3, 0 to disable).
    backoff_base : float
        Base delay in seconds for exponential backoff (default 0.5).
    backoff_max : float
        Maximum backoff delay in seconds (default 30.0).
    flight_timeout : float, optional
        Timeout in seconds for Flight SQL RPC calls.
    """

    def __init__(
        self,
        endpoint: str,
        token: str = "",
        tenant_id: str = "",
        *,
        token_provider: Callable[[], str] | None = None,
        api_base: str = "/api/v1/data",
        rest_timeout: float = 30.0,
        ca_cert: str | None = None,
        insecure: bool = False,
        max_retries: int = 3,
        backoff_base: float = 0.5,
        backoff_max: float = 30.0,
        flight_timeout: float | None = None,
    ) -> None:
        if token and token_provider:
            raise DCHConfigError(
                "Cannot specify both 'token' and 'token_provider'."
                " Please provide either a static token or a token_provider callable, not both."
            )

        rest_url, flight_url = _build_urls(endpoint)

        self._rest = RestClient(
            url=rest_url,
            token=token,
            tenant_id=tenant_id,
            token_provider=token_provider,
            api_base=api_base,
            timeout=rest_timeout,
            ca_cert=ca_cert,
            insecure=insecure,
            max_retries=max_retries,
            backoff_base=backoff_base,
            backoff_max=backoff_max,
        )

        # Built on first use: ``_flight`` imports pyarrow and
        # adbc-driver-flightsql at module scope, which only the ``[flight]``
        # extra installs.  Deferring the import keeps the package importable
        # on a REST-only install.
        self._flight: FlightClient | None = None
        self._flight_kwargs: dict[str, Any] = {
            "url": flight_url,
            "token": token,
            "tenant_id": tenant_id,
            "token_provider": token_provider,
            "timeout": flight_timeout,
            "ca_cert": ca_cert,
            "insecure": insecure,
        }

    def _require_flight(self) -> FlightClient:
        if self._flight is None:
            try:
                from ._flight import FlightClient
            except ImportError as exc:
                raise DCHConfigError(
                    "Flight SQL support requires the 'flight' extra: pip install \"data-connect-hub[flight]\""
                ) from exc
            self._flight = FlightClient(**self._flight_kwargs)
        return self._flight

    # -- context manager --

    def __enter__(self) -> DataConnectClient:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def close(self) -> None:
        """Close underlying clients."""
        self._rest.close()
        if self._flight is not None:
            self._flight.close()

    # -- Connections --

    def list_connections(self) -> list[DataConnection]:
        return self._rest.list_connections()

    def get_connection(self, connection_id: str) -> DataConnection:
        return self._rest.get_connection(connection_id)

    def create_connection(
        self,
        *,
        name: str,
        connection_type_id: str,
        data_format: DataFormat,
        credentials_ref: CredentialsRef,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        req = CreateConnectionRequest(
            name=name,
            data_connection_type_id=connection_type_id,
            format=data_format,
            credentials_ref=credentials_ref,
            properties=properties or {},
        )
        return self._rest.create_connection(req)

    def update_connection(
        self,
        connection_id: str,
        *,
        name: str | None = None,
        connection_type_id: str | None = None,
        data_format: DataFormat | None = None,
        credentials_ref: CredentialsRef | None = None,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        if all(v is None for v in (name, connection_type_id, data_format, credentials_ref, properties)):
            raise DCHConfigError("at least one field must be provided for update")
        req = UpdateConnectionRequest(
            name=name,
            data_connection_type_id=connection_type_id,
            format=data_format,
            credentials_ref=credentials_ref,
            properties=properties,
        )
        return self._rest.update_connection(connection_id, req)

    def delete_connection(self, connection_id: str) -> None:
        self._rest.delete_connection(connection_id)

    # -- Connection Types --

    def list_connection_types(self) -> list[ConnectionType]:
        return self._rest.list_connection_types()

    def get_connection_type(self, type_id: str) -> ConnectionType:
        return self._rest.get_connection_type(type_id)

    def create_connection_type(
        self,
        *,
        name: str,
        provider: str,
        description: str | None = None,
        credentials_fields: list[CredentialField] | None = None,
    ) -> ConnectionType:
        req = CreateConnectionTypeRequest(
            name=name,
            provider=provider,
            description=description,
            credentials_fields=credentials_fields or [],
        )
        return self._rest.create_connection_type(req)

    def update_connection_type(
        self,
        type_id: str,
        *,
        name: str | None = None,
        provider: str | None = None,
        description: str | None = None,
        credentials_fields: list[CredentialField] | None = None,
    ) -> ConnectionType:
        if all(v is None for v in (name, provider, description, credentials_fields)):
            raise DCHConfigError("at least one field must be provided for update")
        req = UpdateConnectionTypeRequest(
            name=name,
            provider=provider,
            description=description,
            credentials_fields=credentials_fields,
        )
        return self._rest.update_connection_type(type_id, req)

    def delete_connection_type(self, type_id: str) -> None:
        self._rest.delete_connection_type(type_id)

    # -- Flight SQL queries --

    def read(self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None) -> pa.Table:
        """Execute *sql* via Flight SQL and return the full result as a PyArrow Table."""
        return self._require_flight().read(sql, connection_id, parameters=parameters)

    def read_batches(
        self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None
    ) -> Generator[pa.RecordBatch, None, None]:
        """Execute *sql* via Flight SQL and return a streaming iterator of RecordBatches.

        Yields one :class:`pyarrow.RecordBatch` per iteration.  The
        underlying cursor and connection are closed automatically when the
        generator is exhausted or closed::

            for batch in client.read_batches("SELECT ...", "conn-1"):
                process(batch)
        """
        return self._require_flight().read_batches(sql, connection_id, parameters=parameters)

    def read_pandas(self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None) -> pd.DataFrame:
        """Execute *sql* via Flight SQL and return the result as a pandas DataFrame."""
        return self._require_flight().read_pandas(sql, connection_id, parameters=parameters)

    def get_tables(
        self,
        connection_id: str,
        *,
        table_name_filter: str | None = None,
        include_schema: bool = False,
    ) -> pa.Table:
        """Retrieve table metadata via Flight SQL ``CommandGetTables``."""
        return self._require_flight().get_tables(
            connection_id, table_name_filter=table_name_filter, include_schema=include_schema
        )

    def server_info(self) -> dict[str, Any]:
        """Return Flight SQL server metadata."""
        return self._require_flight().server_info()
