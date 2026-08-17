"""Unified DataConnectClient for REST and Flight SQL access."""

from __future__ import annotations

from collections.abc import Callable, Sequence
from typing import TYPE_CHECKING, Any

from .exceptions import DCHConfigError
from .models import (
    Admin,
    ConnectionType,
    CreateConnectionRequest,
    CreateConnectionTypeRequest,
    CredentialField,
    DataConnection,
    DataFormat,
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)
from .rest import RestClient

if TYPE_CHECKING:
    import pandas as pd
    import pyarrow as pa

    from .flight import FlightSQLClient


class DataConnectClient:
    """Single entry point for all DCH interactions.

    Parameters
    ----------
    rest_url : str, optional
        Base URL of the DCH REST service.
    flight_url : str, optional
        gRPC endpoint of the DCH Flight SQL service.
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
        rest_url: str | None = None,
        flight_url: str | None = None,
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

        self._rest: RestClient | None = None
        self._flight: FlightSQLClient | None = None

        if rest_url:
            self._rest = RestClient(
                base_url=rest_url,
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

        if flight_url:
            from .flight import FlightSQLClient as _FlightSQLClient

            self._flight = _FlightSQLClient(
                flight_url=flight_url,
                token=token,
                tenant_id=tenant_id,
                token_provider=token_provider,
                timeout=flight_timeout,
                ca_cert=ca_cert,
                insecure=insecure,
            )

    # -- context manager --

    def __enter__(self) -> DataConnectClient:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def close(self) -> None:
        """Close underlying clients."""
        if self._rest:
            self._rest.close()
        if self._flight:
            self._flight.close()

    # -- guards --

    def _require_rest(self) -> RestClient:
        if self._rest is None:
            raise DCHConfigError("rest_url is required for this operation")
        return self._rest

    def _require_flight(self) -> FlightSQLClient:
        if self._flight is None:
            raise DCHConfigError("flight_url is required for this operation")
        return self._flight

    # -- Connections --

    def list_connections(self) -> list[DataConnection]:
        return self._require_rest().list_connections()

    def get_connection(self, connection_id: str) -> DataConnection:
        return self._require_rest().get_connection(connection_id)

    def create_connection(
        self,
        *,
        name: str,
        connection_type_id: str,
        data_format: DataFormat,
        admin: Admin | None = None,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        req = CreateConnectionRequest(
            name=name,
            data_connection_type_id=connection_type_id,
            format=data_format,
            admin=admin,
            properties=properties or {},
        )
        return self._require_rest().create_connection(req)

    def update_connection(
        self,
        connection_id: str,
        *,
        name: str | None = None,
        connection_type_id: str | None = None,
        data_format: DataFormat | None = None,
        admin: Admin | None = None,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        if all(v is None for v in (name, connection_type_id, data_format, admin, properties)):
            raise DCHConfigError("at least one field must be provided for update")
        req = UpdateConnectionRequest(
            name=name,
            data_connection_type_id=connection_type_id,
            format=data_format,
            admin=admin,
            properties=properties,
        )
        return self._require_rest().update_connection(connection_id, req)

    def delete_connection(self, connection_id: str) -> None:
        self._require_rest().delete_connection(connection_id)

    # -- Connection Types --

    def list_connection_types(self) -> list[ConnectionType]:
        return self._require_rest().list_connection_types()

    def get_connection_type(self, type_id: str) -> ConnectionType:
        return self._require_rest().get_connection_type(type_id)

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
        return self._require_rest().create_connection_type(req)

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
        return self._require_rest().update_connection_type(type_id, req)

    def delete_connection_type(self, type_id: str) -> None:
        self._require_rest().delete_connection_type(type_id)

    # -- Flight SQL queries --

    def read(self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None) -> pa.Table:
        """Execute *sql* via Flight SQL and return the full result as a PyArrow Table."""
        return self._require_flight().read(sql, connection_id, parameters=parameters)

    def read_pandas(self, sql: str, connection_id: str, *, parameters: Sequence[Any] | None = None) -> pd.DataFrame:
        """Execute *sql* via Flight SQL and return the result as a pandas DataFrame."""
        return self._require_flight().read_pandas(sql, connection_id, parameters=parameters)

    def server_info(self) -> dict[str, Any]:
        """Return Flight SQL server metadata."""
        return self._require_flight().server_info()
