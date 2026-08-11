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

    def test_flight_without_url_raises(self) -> None:
        client = DataConnectClient()
        with pytest.raises(DCHConfigError, match="flight_url"):
            client.server_info()


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
            connection_type_id="postgres",
            data_format="tabular",
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

    def test_update_connection_with_admin(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient(rest_url="http://localhost")
        assert client._rest is not None
        client._rest.update_connection = MagicMock(return_value=conn)  # type: ignore[method-assign]

        from data_connect_hub.models import AdminSecretRef

        client.update_connection("123", admin=AdminSecretRef(secret_ref="secret/new"))
        req = client._rest.update_connection.call_args[0][1]
        assert req.admin == AdminSecretRef(secret_ref="secret/new")


class TestFlightDelegation:
    def test_read(self) -> None:
        import pyarrow as pa

        table = pa.table({"col": [1, 2]})
        client = DataConnectClient(rest_url="http://localhost")
        client._flight = MagicMock()
        client._flight.read.return_value = table

        result = client.read("SELECT 1", "conn-1")
        assert result.equals(table)
        client._flight.read.assert_called_once_with("SELECT 1", "conn-1", parameters=None)

    def test_read_with_parameters(self) -> None:
        import pyarrow as pa

        table = pa.table({"col": [1]})
        client = DataConnectClient(rest_url="http://localhost")
        client._flight = MagicMock()
        client._flight.read.return_value = table

        client.read("SELECT $1", "conn-1", parameters=[42])
        client._flight.read.assert_called_once_with("SELECT $1", "conn-1", parameters=[42])

    def test_read_pandas(self) -> None:
        import pandas as pd

        df = pd.DataFrame({"col": [1, 2]})
        client = DataConnectClient(rest_url="http://localhost")
        client._flight = MagicMock()
        client._flight.read_pandas.return_value = df

        result = client.read_pandas("SELECT 1", "conn-1")
        assert isinstance(result, pd.DataFrame)
        client._flight.read_pandas.assert_called_once_with("SELECT 1", "conn-1", parameters=None)

    def test_read_pandas_with_parameters(self) -> None:
        import pandas as pd

        df = pd.DataFrame({"col": [1]})
        client = DataConnectClient(rest_url="http://localhost")
        client._flight = MagicMock()
        client._flight.read_pandas.return_value = df

        client.read_pandas("SELECT $1", "conn-1", parameters=[42])
        client._flight.read_pandas.assert_called_once_with("SELECT $1", "conn-1", parameters=[42])

    def test_server_info(self) -> None:
        client = DataConnectClient(rest_url="http://localhost")
        client._flight = MagicMock()
        client._flight.server_info.return_value = {"vendor": "DCH"}

        result = client.server_info()
        assert result == {"vendor": "DCH"}
        client._flight.server_info.assert_called_once()


class TestTokenProviderGuard:
    def test_token_and_provider_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="Cannot specify both"):
            DataConnectClient(
                rest_url="http://localhost",
                token="tok",
                token_provider=lambda: "fresh",
            )

    def test_provider_only_accepted(self) -> None:
        client = DataConnectClient(
            rest_url="http://localhost",
            token_provider=lambda: "fresh",
        )
        assert client._rest is not None
