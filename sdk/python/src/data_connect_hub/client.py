"""Unified DataConnectClient for REST API access."""

from __future__ import annotations

import asyncio
import atexit
import concurrent.futures
import warnings
from collections.abc import Coroutine
from typing import Any, TypeVar

from .exceptions import DCHConfigError
from .models import (
    ConnectionType,
    CreateConnectionRequest,
    CreateConnectionTypeRequest,
    DataConnection,
    DataLocation,
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)
from .rest import RestClient

_T = TypeVar("_T")

_EXECUTOR = concurrent.futures.ThreadPoolExecutor(max_workers=1, thread_name_prefix="dch-sync")
atexit.register(_EXECUTOR.shutdown, wait=True)


def _run_sync(coro: Coroutine[Any, Any, _T]) -> _T:
    """Run an async coroutine synchronously.

    When called from within a running event loop (e.g. Jupyter), this falls
    back to executing the coroutine in a separate thread via a shared pool.
    """
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        loop = None

    if loop is not None and loop.is_running():
        return _EXECUTOR.submit(asyncio.run, coro).result()
    return asyncio.run(coro)


class DataConnectClient:
    """Single entry point for all DCH interactions.

    Provides synchronous methods by default and ``*_async`` variants
    for async callers.

    Parameters
    ----------
    rest_url : str, optional
        Base URL of the DCH REST service.
    token : str
        Raw Bearer token value (without the "Bearer " prefix).
    tenant_id : str
        Tenant identifier sent via ``x-tenant-id`` header.
    api_base : str
        API path prefix (default ``/api/v1/data``).
    timeout : float
        HTTP request timeout in seconds.
    max_retries : int
        Maximum retry attempts for transient errors (default 3, 0 to disable).
    backoff_base : float
        Base delay in seconds for exponential backoff (default 0.5).
    backoff_max : float
        Maximum backoff delay in seconds (default 30.0).
    """

    def __init__(
        self,
        rest_url: str | None = None,
        token: str = "",
        tenant_id: str = "",
        *,
        api_base: str = "/api/v1/data",
        timeout: float = 30.0,
        max_retries: int = 3,
        backoff_base: float = 0.5,
        backoff_max: float = 30.0,
    ) -> None:
        self._rest: RestClient | None = None

        if rest_url:
            self._rest = RestClient(
                base_url=rest_url,
                token=token,
                tenant_id=tenant_id,
                api_base=api_base,
                timeout=timeout,
                max_retries=max_retries,
                backoff_base=backoff_base,
                backoff_max=backoff_max,
            )

    # -- context manager --

    def __enter__(self) -> DataConnectClient:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    async def __aenter__(self) -> DataConnectClient:
        return self

    async def __aexit__(self, *exc: object) -> None:
        await self.close_async()

    def close(self) -> None:
        """Close the underlying HTTP client synchronously.

        If called from within a running event loop (rare), emits a
        ResourceWarning rather than risking a deadlock.
        """
        if not self._rest:
            return
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            loop = None

        if loop is not None and loop.is_running():
            warnings.warn(
                "DataConnectClient.close() called from within a running event loop. "
                "Use 'await client.close_async()' instead. Resources may not be freed.",
                ResourceWarning,
                stacklevel=2,
            )
            return
        asyncio.run(self._rest.close())

    async def close_async(self) -> None:
        if self._rest:
            await self._rest.close()

    # -- guards --

    def _require_rest(self) -> RestClient:
        if self._rest is None:
            raise DCHConfigError("rest_url is required for this operation")
        return self._rest

    # -- Connections (sync) --

    def list_connections(self) -> list[DataConnection]:
        return _run_sync(self._require_rest().list_connections())

    def get_connection(self, connection_id: str) -> DataConnection:
        return _run_sync(self._require_rest().get_connection(connection_id))

    def create_connection(
        self,
        *,
        name: str,
        namespace: str,
        provider: str,
        data_format: str,
        location_url: str,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        req = CreateConnectionRequest(
            name=name,
            namespace=namespace,
            provider=provider,
            format=data_format,
            location=DataLocation(url=location_url),
            properties=properties or {},
        )
        return _run_sync(self._require_rest().create_connection(req))

    def update_connection(
        self,
        connection_id: str,
        *,
        name: str | None = None,
        namespace: str | None = None,
        provider: str | None = None,
        data_format: str | None = None,
        location_url: str | None = None,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        if all(v is None for v in (name, namespace, provider, data_format, location_url, properties)):
            raise DCHConfigError("at least one field must be provided for update")
        location = DataLocation(url=location_url) if location_url is not None else None
        req = UpdateConnectionRequest(
            name=name,
            namespace=namespace,
            provider=provider,
            format=data_format,
            location=location,
            properties=properties,
        )
        return _run_sync(self._require_rest().update_connection(connection_id, req))

    def delete_connection(self, connection_id: str) -> None:
        _run_sync(self._require_rest().delete_connection(connection_id))

    # -- Connections (async) --

    async def list_connections_async(self) -> list[DataConnection]:
        return await self._require_rest().list_connections()

    async def get_connection_async(self, connection_id: str) -> DataConnection:
        return await self._require_rest().get_connection(connection_id)

    async def create_connection_async(
        self,
        *,
        name: str,
        namespace: str,
        provider: str,
        data_format: str,
        location_url: str,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        req = CreateConnectionRequest(
            name=name,
            namespace=namespace,
            provider=provider,
            format=data_format,
            location=DataLocation(url=location_url),
            properties=properties or {},
        )
        return await self._require_rest().create_connection(req)

    async def update_connection_async(
        self,
        connection_id: str,
        *,
        name: str | None = None,
        namespace: str | None = None,
        provider: str | None = None,
        data_format: str | None = None,
        location_url: str | None = None,
        properties: dict[str, str] | None = None,
    ) -> DataConnection:
        if all(v is None for v in (name, namespace, provider, data_format, location_url, properties)):
            raise DCHConfigError("at least one field must be provided for update")
        location = DataLocation(url=location_url) if location_url is not None else None
        req = UpdateConnectionRequest(
            name=name,
            namespace=namespace,
            provider=provider,
            format=data_format,
            location=location,
            properties=properties,
        )
        return await self._require_rest().update_connection(connection_id, req)

    async def delete_connection_async(self, connection_id: str) -> None:
        await self._require_rest().delete_connection(connection_id)

    # -- Connection Types (sync) --

    def list_connection_types(self) -> list[ConnectionType]:
        return _run_sync(self._require_rest().list_connection_types())

    def get_connection_type(self, type_id: str) -> ConnectionType:
        return _run_sync(self._require_rest().get_connection_type(type_id))

    def create_connection_type(
        self,
        *,
        name: str,
        description: str = "",
        properties_schema: dict[str, Any] | None = None,
    ) -> ConnectionType:
        req = CreateConnectionTypeRequest(
            name=name,
            description=description,
            properties_schema=properties_schema or {},
        )
        return _run_sync(self._require_rest().create_connection_type(req))

    def update_connection_type(
        self,
        type_id: str,
        *,
        name: str | None = None,
        description: str | None = None,
        properties_schema: dict[str, Any] | None = None,
    ) -> ConnectionType:
        if all(v is None for v in (name, description, properties_schema)):
            raise DCHConfigError("at least one field must be provided for update")
        req = UpdateConnectionTypeRequest(
            name=name,
            description=description,
            properties_schema=properties_schema,
        )
        return _run_sync(self._require_rest().update_connection_type(type_id, req))

    def delete_connection_type(self, type_id: str) -> None:
        _run_sync(self._require_rest().delete_connection_type(type_id))

    # -- Connection Types (async) --

    async def list_connection_types_async(self) -> list[ConnectionType]:
        return await self._require_rest().list_connection_types()

    async def get_connection_type_async(self, type_id: str) -> ConnectionType:
        return await self._require_rest().get_connection_type(type_id)

    async def create_connection_type_async(
        self,
        *,
        name: str,
        description: str = "",
        properties_schema: dict[str, Any] | None = None,
    ) -> ConnectionType:
        req = CreateConnectionTypeRequest(
            name=name,
            description=description,
            properties_schema=properties_schema or {},
        )
        return await self._require_rest().create_connection_type(req)

    async def update_connection_type_async(
        self,
        type_id: str,
        *,
        name: str | None = None,
        description: str | None = None,
        properties_schema: dict[str, Any] | None = None,
    ) -> ConnectionType:
        if all(v is None for v in (name, description, properties_schema)):
            raise DCHConfigError("at least one field must be provided for update")
        req = UpdateConnectionTypeRequest(
            name=name,
            description=description,
            properties_schema=properties_schema,
        )
        return await self._require_rest().update_connection_type(type_id, req)

    async def delete_connection_type_async(self, type_id: str) -> None:
        await self._require_rest().delete_connection_type(type_id)

    # -- Unstructured ingestion --

    def ingest(self, connection_id: str) -> bytes:
        return _run_sync(self._require_rest().ingest(connection_id))

    async def ingest_async(self, connection_id: str) -> bytes:
        return await self._require_rest().ingest(connection_id)
