"""Tests for the FlightSQLClient wrapper."""

from __future__ import annotations

from typing import Any
from unittest.mock import MagicMock, patch

import pandas as pd
import pyarrow as pa
import pytest

from data_connect_hub.exceptions import DCHConfigError, DCHConnectionError, DCHQueryError
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


def _set_mock_flight(mock_flight: MagicMock, connectors: list[str] | None = None) -> MagicMock:
    names = connectors or ["postgres"]
    table = pa.table({"name": names, "description": [""] * len(names)})
    sink = pa.BufferOutputStream()
    writer = pa.ipc.new_stream(sink, table.schema)
    writer.write_table(table)
    writer.close()

    mock_client = MagicMock()
    mock_flight.connect.return_value = mock_client
    mock_result = MagicMock()
    mock_result.body.to_pybytes.return_value = sink.getvalue().to_pybytes()
    mock_client.do_action.return_value = [mock_result]
    return mock_client


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
    @patch("data_connect_hub.flight.flight")
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_returns_dict_with_connectors(
        self, mock_dbapi: MagicMock, mock_flight: MagicMock, flight_client: FlightSQLClient
    ) -> None:
        _set_mock_exceptions(mock_dbapi)
        info: dict[str, Any] = {"vendor": "DCH", "version": "1.0"}
        mock_conn = MagicMock()
        mock_conn.adbc_get_info.return_value = info
        mock_dbapi.connect.return_value = mock_conn
        _set_mock_flight(mock_flight, connectors=["postgres", "sqlite"])

        result = flight_client.server_info()
        assert result["vendor"] == "DCH"
        assert result["version"] == "1.0"
        assert result["supported_connectors"] == ["postgres", "sqlite"]
        mock_conn.close.assert_called_once()

    @patch("data_connect_hub.flight.flight")
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_server_info_connectors_error_propagates(
        self, mock_dbapi: MagicMock, mock_flight: MagicMock, flight_client: FlightSQLClient
    ) -> None:
        _set_mock_exceptions(mock_dbapi)
        info: dict[str, Any] = {"vendor": "DCH", "version": "1.0"}
        mock_conn = MagicMock()
        mock_conn.adbc_get_info.return_value = info
        mock_dbapi.connect.return_value = mock_conn

        mock_flight_client = _set_mock_flight(mock_flight)
        mock_flight_client.do_action.side_effect = Exception("action failed")

        with pytest.raises(DCHConnectionError, match="action failed"):
            flight_client.server_info()
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


class TestTokenProviderGuard:
    def test_token_and_provider_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="Cannot specify both"):
            FlightSQLClient(
                flight_url="grpc://localhost:50051",
                token="tok",
                token_provider=lambda: "fresh",
            )


class TestTokenProvider:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_provider_called_once_and_cached(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=provider,
        )
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        client.read("SELECT 1", "conn-1")
        kwargs1 = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert kwargs1["adbc.flight.sql.rpc.call_header.authorization"] == "Bearer token-1"

        client.read("SELECT 1", "conn-1")
        kwargs2 = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert kwargs2["adbc.flight.sql.rpc.call_header.authorization"] == "Bearer token-1"
        assert call_count == 1

    @patch("data_connect_hub.flight.flight")
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_provider_used_for_server_info(self, mock_dbapi: MagicMock, mock_flight: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=lambda: "fresh-token",
        )
        mock_conn = MagicMock()
        mock_conn.adbc_get_info.return_value = {"vendor": "DCH"}
        mock_dbapi.connect.return_value = mock_conn
        _set_mock_flight(mock_flight)

        client.server_info()

        db_kwargs = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert db_kwargs["adbc.flight.sql.rpc.call_header.authorization"] == "Bearer fresh-token"

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_provider_with_timeout(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=lambda: "fresh-token",
            timeout=10.0,
        )
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        client.read("SELECT 1", "conn-1")

        db_kwargs = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert db_kwargs["adbc.flight.sql.rpc.call_header.authorization"] == "Bearer fresh-token"
        assert db_kwargs["adbc.flight.sql.rpc.timeout_seconds.query"] == "10.0"
        assert db_kwargs["adbc.flight.sql.rpc.timeout_seconds.fetch"] == "10.0"

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_connect_auth_error_triggers_refresh_and_retry(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=provider,
        )
        table = pa.table({"col": [1]})
        mock_conn_ok = MagicMock()
        mock_conn_ok.cursor.return_value = _mock_cursor(table)

        mock_dbapi.connect.side_effect = [
            _OperationalError("UNAUTHENTICATED: token expired"),
            mock_conn_ok,
        ]

        result = client.read("SELECT 1", "conn-1")

        assert call_count == 2
        assert result.equals(table)

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_query_auth_error_not_retried(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=provider,
        )

        cursor_fail = MagicMock()
        cursor_fail.execute.side_effect = _Error("UNAUTHENTICATED: token expired")
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = cursor_fail
        mock_dbapi.connect.return_value = mock_conn

        with pytest.raises(DCHQueryError, match="UNAUTHENTICATED"):
            client.read("SELECT 1", "conn-1")
        assert call_count == 1

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_auth_error_after_refresh_raises(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=lambda: "bad-token",
        )
        mock_dbapi.connect.side_effect = _OperationalError("UNAUTHENTICATED: invalid")

        with pytest.raises(DCHConnectionError, match="UNAUTHENTICATED"):
            client.read("SELECT 1", "conn-1")

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_non_auth_error_not_retried(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=provider,
        )
        mock_dbapi.connect.side_effect = _OperationalError("connection refused")

        with pytest.raises(DCHConnectionError, match="connection refused"):
            client.read("SELECT 1", "conn-1")
        assert call_count == 1

    @patch("data_connect_hub.flight.flight")
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_server_info_auth_error_triggers_refresh(self, mock_dbapi: MagicMock, mock_flight: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        client = FlightSQLClient(
            flight_url="grpc://localhost:50051",
            tenant_id="t1",
            token_provider=provider,
        )
        mock_conn_ok = MagicMock()
        mock_conn_ok.adbc_get_info.return_value = {"vendor": "DCH"}

        mock_dbapi.connect.side_effect = [
            _OperationalError("UNAUTHENTICATED: token expired"),
            mock_conn_ok,
        ]

        _set_mock_flight(mock_flight)

        result = client.server_info()

        assert call_count == 2
        assert result["vendor"] == "DCH"


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

    @patch("data_connect_hub.flight.flight")
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_timeouts_applied_to_server_info(self, mock_dbapi: MagicMock, mock_flight: MagicMock) -> None:
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
        _set_mock_flight(mock_flight)

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


class TestTLS:
    @patch("data_connect_hub.flight.flight_dbapi")
    def test_insecure_sets_tls_skip_verify(self, mock_dbapi: MagicMock) -> None:
        _set_mock_exceptions(mock_dbapi)
        client = FlightSQLClient(
            flight_url="grpc+tls://localhost:50051",
            token="tok",
            tenant_id="t1",
            insecure=True,
        )
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        client.read("SELECT 1", "conn-1")

        db_kwargs = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert db_kwargs["adbc.flight.sql.client_option.tls_skip_verify"] == "true"

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_ca_cert_sets_tls_root_certs(self, mock_dbapi: MagicMock, tmp_path: Any) -> None:
        _set_mock_exceptions(mock_dbapi)
        cert_file = tmp_path / "ca.pem"
        cert_file.write_text("-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----\n")

        client = FlightSQLClient(
            flight_url="grpc+tls://localhost:50051",
            token="tok",
            tenant_id="t1",
            ca_cert=str(cert_file),
        )
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        client.read("SELECT 1", "conn-1")

        db_kwargs = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert "-----BEGIN CERTIFICATE-----" in db_kwargs["adbc.flight.sql.client_option.tls_root_certs"]

    def test_ca_cert_file_not_found_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="CA certificate file not found"):
            FlightSQLClient(
                flight_url="grpc+tls://localhost:50051",
                token="tok",
                tenant_id="t1",
                ca_cert="/nonexistent/ca.pem",
            )

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_insecure_overrides_ca_cert(self, mock_dbapi: MagicMock, tmp_path: Any) -> None:
        _set_mock_exceptions(mock_dbapi)
        cert_file = tmp_path / "ca.pem"
        cert_file.write_text("-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----\n")

        client = FlightSQLClient(
            flight_url="grpc+tls://localhost:50051",
            token="tok",
            tenant_id="t1",
            ca_cert=str(cert_file),
            insecure=True,
        )
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        client.read("SELECT 1", "conn-1")

        db_kwargs = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert db_kwargs["adbc.flight.sql.client_option.tls_skip_verify"] == "true"
        assert "adbc.flight.sql.client_option.tls_root_certs" not in db_kwargs

    @patch("data_connect_hub.flight.flight_dbapi")
    def test_no_tls_options_by_default(self, mock_dbapi: MagicMock, flight_client: FlightSQLClient) -> None:
        _set_mock_exceptions(mock_dbapi)
        table = pa.table({"col": [1]})
        mock_conn = MagicMock()
        mock_conn.cursor.return_value = _mock_cursor(table)
        mock_dbapi.connect.return_value = mock_conn

        flight_client.read("SELECT 1", "conn-1")

        db_kwargs = mock_dbapi.connect.call_args.kwargs["db_kwargs"]
        assert "adbc.flight.sql.client_option.tls_skip_verify" not in db_kwargs
        assert "adbc.flight.sql.client_option.tls_root_certs" not in db_kwargs


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
