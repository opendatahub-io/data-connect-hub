"""Tests for the unified DataConnectClient."""

from __future__ import annotations

from unittest.mock import MagicMock

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


class TestConnectionsDelegation:
    def test_list_connections(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.list_connections = MagicMock(return_value=[])  # type: ignore[method-assign]

        result = client.list_connections()
        assert result == []
        client._rest.list_connections.assert_called_once()

    def test_get_connection(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.get_connection = MagicMock(return_value=conn)  # type: ignore[method-assign]

        result = client.get_connection("123")
        assert result.id == "123"

    def test_create_connection(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.create_connection = MagicMock(return_value=conn)  # type: ignore[method-assign]

        result = client.create_connection(
            name="test-conn",
            namespace="test-ns",
            provider="postgres",
            data_format="tabular",
            location_url="postgresql://localhost:5432/db",
        )
        assert result.id == "123"

    def test_delete_connection(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.delete_connection = MagicMock(return_value=None)  # type: ignore[method-assign]

        client.delete_connection("123")
        client._rest.delete_connection.assert_called_once_with("123")


class TestEmptyUpdateGuards:
    def test_update_connection_no_fields_raises(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            client.update_connection("123")

    def test_update_connection_type_no_fields_raises(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            client.update_connection_type("ct-1")

    def test_update_connection_empty_location_url(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.update_connection = MagicMock(return_value=conn)  # type: ignore[method-assign]

        client.update_connection("123", location_url="")
        req = client._rest.update_connection.call_args[0][1]
        assert req.location is not None
        assert req.location.url == ""


class TestIngestDelegation:
    async def test_ingest(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.ingest = MagicMock(return_value=b"data")  # type: ignore[method-assign]

        result = await client.ingest("conn-1")
        assert result == b"data"
