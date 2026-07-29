"""Tests for the unified DataConnectClient."""

from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from data_connect_hub.client import DataConnectClient
from data_connect_hub.exceptions import DCHConfigError

from .conftest import SAMPLE_CONNECTION_JSON


class TestConfigGuards:
    def test_rest_without_url_raises(self) -> None:
        client = DataConnectClient()
        with pytest.raises(DCHConfigError, match="rest_url"):
            client.list_connections()


class TestContextManager:
    def test_sync_context_manager(self) -> None:
        with DataConnectClient(rest_url="http://localhost") as client:
            assert client is not None

    async def test_async_context_manager(self) -> None:
        async with DataConnectClient(rest_url="http://localhost") as client:
            assert client is not None


class TestConnectionsDelegation:
    async def test_list_connections_async(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.list_connections = AsyncMock(return_value=[])  # type: ignore[method-assign]

        result = await client.list_connections_async()
        assert result == []
        client._rest.list_connections.assert_awaited_once()

    async def test_get_connection_async(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.get_connection = AsyncMock(return_value=conn)  # type: ignore[method-assign]

        result = await client.get_connection_async("123")
        assert result.id == "123"

    async def test_create_connection_async(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.create_connection = AsyncMock(return_value=conn)  # type: ignore[method-assign]

        result = await client.create_connection_async(
            name="test-conn",
            namespace="test-ns",
            provider="postgres",
            data_format="jdbc",
            location_url="postgresql://localhost:5432/db",
        )
        assert result.id == "123"

    async def test_delete_connection_async(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.delete_connection = AsyncMock(return_value=None)  # type: ignore[method-assign]

        await client.delete_connection_async("123")
        client._rest.delete_connection.assert_awaited_once_with("123")


class TestEmptyUpdateGuards:
    def test_update_connection_no_fields_raises(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            client.update_connection("123")

    async def test_update_connection_async_no_fields_raises(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            await client.update_connection_async("123")

    def test_update_connection_type_no_fields_raises(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            client.update_connection_type("ct-1")

    async def test_update_connection_type_async_no_fields_raises(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            await client.update_connection_type_async("ct-1")


class TestIngestDelegation:
    async def test_ingest_async(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.ingest = AsyncMock(return_value=b"data")  # type: ignore[method-assign]

        result = await client.ingest_async("conn-1")
        assert result == b"data"
