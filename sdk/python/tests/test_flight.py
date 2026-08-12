"""Tests for the FlightSQLClient wrapper."""

from __future__ import annotations

from typing import Any
from unittest.mock import MagicMock, patch

import pandas as pd
import pyarrow as pa
import pytest

from data_connect_hub.exceptions import DCHConnectionError, DCHQueryError
from data_connect_hub.flight import FlightSQLClient


class _Error(Exception):
    pass


class _OperationalError(_Error):
    pass


class _InterfaceError(_Error):
    pass


class _ProgrammingError(_Error):
    pass


@pytest.fixture()
def flight_client() -> FlightSQLClient:
    return FlightSQLClient(flight_url="grpc://localhost:50051", token="tok", tenant_id="t1")


def _mock_cursor(table: pa.Table) -> MagicMock:
    cursor = MagicMock()
    cursor.fetch_arrow_table.return_value = table
    return cursor


def _set_mock_exceptions(mock_dbapi: MagicMock) -> None:
    mock_dbapi.Error = _Error
    mock_dbapi.InterfaceError = _InterfaceError
    mock_dbapi.OperationalError = _OperationalError
    mock_dbapi.ProgrammingError = _ProgrammingError


class TestRead:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_returns_table(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1, 2, 3]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        result = flight_client.read("SELECT 1", "conn-1")
        assert result.equals(table)
        mock_conn.close.assert_called_once()

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_empty_result_returns_empty_table(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        empty = pa.table({"col": pa.array([], type=pa.int64())})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(empty)
        mock_dbapi.connect.return_value = mock_conn

        result = flight_client.read("SELECT 1", "conn-1")
        assert result.num_rows == 0

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_operational_error_mapped(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        mock_conn = MagicMock()
        cursor = MagicMock()
        cursor.execute.side_effect = _OperationalError("bad sql")
        mock_conn.cursor.return_value = cursor
        mock_dbapi.connect.return_value = mock_conn

        with pytest.raises(DCHQueryError, match="bad sql"):
            flight_client.read("BAD SQL", "conn-1")

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_programming_error_mapped(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        mock_conn = MagicMock()
        cursor = MagicMock()
        cursor.execute.side_effect = _ProgrammingError("syntax error")
        mock_conn.cursor.return_value = cursor
        mock_dbapi.connect.return_value = mock_conn

        with pytest.raises(DCHQueryError, match="syntax error"):
            flight_client.read("BAD SQL", "conn-1")


class TestReadPandas:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_returns_dataframe(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1, 2, 3]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        result = flight_client.read_pandas("SELECT 1", "conn-1")
        assert isinstance(result, pd.DataFrame)
        assert list(result["col"]) == [1, 2, 3]
        mock_conn.close.assert_called_once()

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_empty_result_returns_empty_dataframe(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        empty = pa.table({"col": pa.array([], type=pa.int64())})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(empty)
        mock_dbapi.connect.return_value = mock_conn

        result = flight_client.read_pandas("SELECT 1", "conn-1")
        assert isinstance(result, pd.DataFrame)
        assert len(result) == 0


class TestServerInfo:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_returns_dict(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        info: dict[str, Any] = {"vendor": "DCH", "version": "1.0"}
        mock_conn = MagicMock()
        mock_conn.adbc_get_info.return_value = info
        mock_dbapi.connect.return_value = mock_conn

        result = flight_client.server_info()
        assert result == info
        mock_conn.close.assert_called_once()


class TestConnectionError:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_interface_error_mapped(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        mock_dbapi.connect.side_effect = _InterfaceError("unreachable")

        with pytest.raises(DCHConnectionError, match="unreachable"):
            flight_client.read("SELECT 1", "conn-1")

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_operational_error_on_connect_mapped(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        mock_dbapi.connect.side_effect = _OperationalError("connection refused")

        with pytest.raises(DCHConnectionError, match="connection refused"):
            flight_client.read("SELECT 1", "conn-1")

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_server_info_connect_error_mapped(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        mock_dbapi.connect.side_effect = _InterfaceError("unreachable")

        with pytest.raises(DCHConnectionError, match="unreachable"):
            flight_client.server_info()

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_server_info_operational_error_mapped(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        mock_conn = MagicMock()
        mock_conn.adbc_get_info.side_effect = _OperationalError("connection refused")
        mock_dbapi.connect.return_value = mock_conn

        with pytest.raises(DCHConnectionError, match="connection refused"):
            flight_client.server_info()


class TestHeaders:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_connection_id_injected(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        flight_client.read("SELECT 1", "my-conn")

        call_kwargs = mock_dbapi.connect.call_args
        db_kwargs = call_kwargs.kwargs.get("db_kwargs", call_kwargs[1].get("db_kwargs", {}))
        assert db_kwargs["adbc.flight.sql.rpc.call_header.x-data-connection-id"] == "my-conn"
        assert db_kwargs["adbc.flight.sql.rpc.call_header.authorization"] == "Bearer tok"
        assert db_kwargs["adbc.flight.sql.rpc.call_header.x-tenant-id"] == "t1"


class TestTimeouts:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_timeouts_injected(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            token="tok",
            tenant_id="t1",
            timeout=10.0,
        )
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        client.read("SELECT 1", "conn-1")

        db_kwargs = mock_dbapi.connect.call_args.kwargs.get("db_kwargs", {})
        assert db_kwargs["adbc.flight.sql.rpc.timeout_seconds.query"] == "10.0"
        assert db_kwargs["adbc.flight.sql.rpc.timeout_seconds.fetch"] == "10.0"

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_timeouts_applied_to_server_info(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            token="tok",
            tenant_id="t1",
            timeout=10.0,
        )
        mock_conn = MagicMock()
        mock_conn.adbc_get_info.return_value = {"vendor": "DCH"}
        mock_dbapi.connect.return_value = mock_conn

        client.server_info()

        db_kwargs = mock_dbapi.connect.call_args.kwargs.get("db_kwargs", {})
        assert db_kwargs["adbc.flight.sql.rpc.timeout_seconds.query"] == "10.0"
        assert db_kwargs["adbc.flight.sql.rpc.timeout_seconds.fetch"] == "10.0"

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_no_timeouts_by_default(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        flight_client.read("SELECT 1", "conn-1")

        db_kwargs = mock_dbapi.connect.call_args.kwargs.get("db_kwargs", {})
        assert "adbc.flight.sql.rpc.timeout_seconds.query" not in db_kwargs
        assert "adbc.flight.sql.rpc.timeout_seconds.fetch" not in db_kwargs


class TestParameters:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_parameters_forwarded(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1]})
        cursor = _mock_cursor(table)
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = cursor
        mock_dbapi.connect.return_value = mock_conn

        params = [42]
        flight_client.read("SELECT $1", "conn-1", parameters=params)

        cursor.execute.assert_called_once_with("SELECT $1", [42])

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_none_parameters_forwarded(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1]})
        cursor = _mock_cursor(table)
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = cursor
        mock_dbapi.connect.return_value = mock_conn

        flight_client.read("SELECT 1", "conn-1")

        cursor.execute.assert_called_once_with("SELECT 1", None)

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_read_pandas_forwards_parameters(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1]})
        cursor = _mock_cursor(table)
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = cursor
        mock_dbapi.connect.return_value = mock_conn

        result = flight_client.read_pandas("SELECT $1", "conn-1", parameters=[42])

        assert isinstance(result, pd.DataFrame)
        cursor.execute.assert_called_once_with("SELECT $1", [42])
