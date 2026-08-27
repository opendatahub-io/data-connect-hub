"""Tests for the unified DataConnectClient."""

from __future__ import annotations

import sys
from unittest.mock import MagicMock

import pytest

from data_connect_hub.client import DataConnectClient, _build_urls
from data_connect_hub.exceptions import DCHConfigError

from .conftest import SAMPLE_CONNECTION_JSON


class TestContextManager:
    def test_sync_context_manager(self) -> None:
        with DataConnectClient("localhost") as client:
            assert client is not None


class TestConnectionsDelegation:
    def test_list_connections(self) -> None:
        client = DataConnectClient("localhost")
        client._rest.list_connections = MagicMock(return_value=[])  # type: ignore[method-assign]

        result = client.list_connections()
        assert result == []
        client._rest.list_connections.assert_called_once()

    def test_get_connection(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient("localhost")
        client._rest.get_connection = MagicMock(return_value=conn)  # type: ignore[method-assign]

        result = client.get_connection("123")
        assert result.id == "123"

    def test_create_connection(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient("localhost")
        client._rest.create_connection = MagicMock(return_value=conn)  # type: ignore[method-assign]

        result = client.create_connection(
            name="test-conn",
            connection_type_id="postgres",
            data_format="tabular",
        )
        assert result.id == "123"

    def test_delete_connection(self) -> None:
        client = DataConnectClient("localhost")
        client._rest.delete_connection = MagicMock(return_value=None)  # type: ignore[method-assign]

        client.delete_connection("123")
        client._rest.delete_connection.assert_called_once_with("123")


class TestEmptyUpdateGuards:
    def test_update_connection_no_fields_raises(self) -> None:
        client = DataConnectClient("localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            client.update_connection("123")

    def test_update_connection_type_no_fields_raises(self) -> None:
        client = DataConnectClient("localhost")
        with pytest.raises(DCHConfigError, match="at least one field"):
            client.update_connection_type("ct-1")

    def test_update_connection_with_admin(self) -> None:
        from data_connect_hub.models import DataConnection

        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        client = DataConnectClient("localhost")
        client._rest.update_connection = MagicMock(return_value=conn)  # type: ignore[method-assign]

        from data_connect_hub.models import AdminSecretRef

        client.update_connection("123", admin=AdminSecretRef(secret_ref="secret/new"))
        req = client._rest.update_connection.call_args[0][1]
        assert req.admin == AdminSecretRef(secret_ref="secret/new")


class TestFlightDelegation:
    def test_read(self) -> None:
        import pyarrow as pa

        table = pa.table({"col": [1, 2]})
        client = DataConnectClient("localhost")
        client._flight = MagicMock()
        client._flight.read.return_value = table

        result = client.read("SELECT 1", "conn-1")
        assert result.equals(table)
        client._flight.read.assert_called_once_with("SELECT 1", "conn-1", parameters=None)

    def test_read_with_parameters(self) -> None:
        import pyarrow as pa

        table = pa.table({"col": [1]})
        client = DataConnectClient("localhost")
        client._flight = MagicMock()
        client._flight.read.return_value = table

        client.read("SELECT $1", "conn-1", parameters=[42])
        client._flight.read.assert_called_once_with("SELECT $1", "conn-1", parameters=[42])

    def test_read_batches(self) -> None:
        stream = MagicMock()
        client = DataConnectClient("localhost")
        client._flight = MagicMock()
        client._flight.read_batches.return_value = stream

        result = client.read_batches("SELECT 1", "conn-1")
        assert result is stream
        client._flight.read_batches.assert_called_once_with("SELECT 1", "conn-1", parameters=None)

    def test_read_batches_with_parameters(self) -> None:
        client = DataConnectClient("localhost")
        client._flight = MagicMock()

        client.read_batches("SELECT $1", "conn-1", parameters=[42])
        client._flight.read_batches.assert_called_once_with("SELECT $1", "conn-1", parameters=[42])

    def test_read_pandas(self) -> None:
        import pandas as pd

        df = pd.DataFrame({"col": [1, 2]})
        client = DataConnectClient("localhost")
        client._flight = MagicMock()
        client._flight.read_pandas.return_value = df

        result = client.read_pandas("SELECT 1", "conn-1")
        assert isinstance(result, pd.DataFrame)
        client._flight.read_pandas.assert_called_once_with("SELECT 1", "conn-1", parameters=None)

    def test_read_pandas_with_parameters(self) -> None:
        import pandas as pd

        df = pd.DataFrame({"col": [1]})
        client = DataConnectClient("localhost")
        client._flight = MagicMock()
        client._flight.read_pandas.return_value = df

        client.read_pandas("SELECT $1", "conn-1", parameters=[42])
        client._flight.read_pandas.assert_called_once_with("SELECT $1", "conn-1", parameters=[42])

    def test_server_info(self) -> None:
        client = DataConnectClient("localhost")
        client._flight = MagicMock()
        client._flight.server_info.return_value = {"vendor": "DCH"}

        result = client.server_info()
        assert result == {"vendor": "DCH"}
        client._flight.server_info.assert_called_once()


class TestTokenProviderGuard:
    def test_token_and_provider_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="Cannot specify both"):
            DataConnectClient(
                "localhost",
                token="tok",
                token_provider=lambda: "fresh",
            )

    def test_provider_only_accepted(self) -> None:
        client = DataConnectClient(
            "localhost",
            token_provider=lambda: "fresh",
        )
        assert client._rest._token_cache is not None
        assert client._flight_kwargs["token_provider"] is not None


class TestDerivedUrls:
    """The single *endpoint* must reach both sub-clients as the right scheme."""

    def test_rest_client_gets_https_url(self) -> None:
        client = DataConnectClient("gateway.example.com:8443")
        assert client._rest._base_url == "https://gateway.example.com:8443"

    def test_endpoint_accepted_as_keyword(self) -> None:
        client = DataConnectClient(endpoint="gateway.example.com:8443")
        assert client._rest._base_url == "https://gateway.example.com:8443"
        assert client._flight_kwargs["url"] == "grpc+tls://gateway.example.com:8443"

    def test_flight_client_gets_grpc_tls_url(self) -> None:
        client = DataConnectClient("gateway.example.com:8443")
        assert client._flight_kwargs["url"] == "grpc+tls://gateway.example.com:8443"
        assert client._require_flight()._url == "grpc+tls://gateway.example.com:8443"

    def test_scheme_in_input_is_normalized_to_tls(self) -> None:
        client = DataConnectClient("http://localhost:8080")
        assert client._rest._base_url == "https://localhost:8080"
        assert client._require_flight()._url == "grpc+tls://localhost:8080"


class TestLazyFlightClient:
    """``_flight`` is only built on first use so REST-only installs can import."""

    def test_not_constructed_eagerly(self) -> None:
        client = DataConnectClient("localhost")
        assert client._flight is None

    def test_constructed_on_first_use(self) -> None:
        client = DataConnectClient("localhost")
        first = client._require_flight()
        assert client._flight is first
        assert client._require_flight() is first

    def test_close_without_flight_use(self) -> None:
        client = DataConnectClient("localhost")
        client.close()  # must not build a FlightClient just to close it
        assert client._flight is None

    def test_close_after_flight_use(self) -> None:
        client = DataConnectClient("localhost")
        flight = MagicMock()
        client._flight = flight
        client.close()
        flight.close.assert_called_once()

    def test_missing_extra_raises_config_error(self, monkeypatch: pytest.MonkeyPatch) -> None:
        import builtins

        real_import = builtins.__import__

        def fake_import(name: str, *args: object, **kwargs: object) -> object:
            if name.split(".")[0] in {"adbc_driver_flightsql", "pyarrow"}:
                raise ModuleNotFoundError(f"No module named {name!r}")
            return real_import(name, *args, **kwargs)  # type: ignore[arg-type]

        for mod in [m for m in sys.modules if m.split(".")[0] in {"adbc_driver_flightsql", "pyarrow"}]:
            monkeypatch.delitem(sys.modules, mod)
        monkeypatch.delitem(sys.modules, "data_connect_hub._flight", raising=False)
        monkeypatch.setattr(builtins, "__import__", fake_import)

        client = DataConnectClient("localhost")
        with pytest.raises(DCHConfigError, match="requires the 'flight' extra"):
            client.server_info()

    def test_core_only_package_import_without_flight_extra(self, monkeypatch: pytest.MonkeyPatch) -> None:
        import builtins
        import importlib

        real_import = builtins.__import__
        blocked = {"adbc_driver_flightsql", "pyarrow"}

        def fake_import(name: str, *args: object, **kwargs: object) -> object:
            if name.split(".")[0] in blocked:
                raise ModuleNotFoundError(f"No module named {name!r}")
            return real_import(name, *args, **kwargs)  # type: ignore[arg-type]

        for mod in list(sys.modules):
            if mod.split(".")[0] in blocked:
                monkeypatch.delitem(sys.modules, mod, raising=False)
            if mod == "data_connect_hub" or mod.startswith("data_connect_hub."):
                monkeypatch.delitem(sys.modules, mod, raising=False)

        monkeypatch.setattr(builtins, "__import__", fake_import)

        import data_connect_hub

        importlib.reload(data_connect_hub)

        client = data_connect_hub.DataConnectClient("localhost")
        with pytest.raises(data_connect_hub.DCHConfigError, match="requires the 'flight' extra"):
            list(client.read_batches("SELECT 1", "conn-1"))


class TestBuildUrls:
    def test_bare_host(self) -> None:
        rest, flight = _build_urls("gateway.example.com")
        assert rest == "https://gateway.example.com"
        assert flight == "grpc+tls://gateway.example.com"

    def test_host_with_port(self) -> None:
        rest, flight = _build_urls("gateway.example.com:8443")
        assert rest == "https://gateway.example.com:8443"
        assert flight == "grpc+tls://gateway.example.com:8443"

    def test_explicit_https_scheme(self) -> None:
        rest, flight = _build_urls("https://gateway.example.com:8443")
        assert rest == "https://gateway.example.com:8443"
        assert flight == "grpc+tls://gateway.example.com:8443"

    def test_explicit_grpc_tls_scheme(self) -> None:
        rest, flight = _build_urls("grpc+tls://gateway.example.com:8443")
        assert rest == "https://gateway.example.com:8443"
        assert flight == "grpc+tls://gateway.example.com:8443"

    @pytest.mark.parametrize("scheme", ["http", "grpc", "HTTP"])
    def test_plaintext_scheme_is_normalized_to_tls(self, scheme: str) -> None:
        """Only TLS is supported; a plaintext scheme in the input is discarded."""
        rest, flight = _build_urls(f"{scheme}://localhost:8080")
        assert rest == "https://localhost:8080"
        assert flight == "grpc+tls://localhost:8080"

    def test_ipv6_literal_keeps_brackets(self) -> None:
        rest, flight = _build_urls("[::1]:8443")
        assert rest == "https://[::1]:8443"
        assert flight == "grpc+tls://[::1]:8443"

    def test_ipv6_literal_without_port(self) -> None:
        rest, flight = _build_urls("https://[2001:db8::1]")
        assert rest == "https://[2001:db8::1]"
        assert flight == "grpc+tls://[2001:db8::1]"

    def test_ipv4_literal(self) -> None:
        rest, flight = _build_urls("127.0.0.1:8443")
        assert rest == "https://127.0.0.1:8443"
        assert flight == "grpc+tls://127.0.0.1:8443"

    def test_out_of_range_port_raises_config_error(self) -> None:
        with pytest.raises(DCHConfigError, match="invalid port"):
            _build_urls("gateway.example.com:99999")

    def test_zero_port_raises_config_error(self) -> None:
        """``:0`` must fail loudly rather than silently defaulting to 443."""
        with pytest.raises(DCHConfigError, match="invalid port"):
            _build_urls("gateway.example.com:0")

    def test_non_numeric_port_raises_config_error(self) -> None:
        with pytest.raises(DCHConfigError, match="invalid port"):
            _build_urls("gateway.example.com:https")

    def test_path_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="must not contain a path"):
            _build_urls("gateway.example.com:8443/api/v1/data")

    def test_credentials_raise_without_echoing_url(self) -> None:
        with pytest.raises(DCHConfigError, match="must not contain credentials") as excinfo:
            _build_urls("https://user:sekret@gateway.example.com")
        assert "sekret" not in str(excinfo.value)

    def test_query_string_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="query string or fragment"):
            _build_urls("gateway.example.com?tenant=foo")

    def test_strips_trailing_slash(self) -> None:
        rest, flight = _build_urls("gateway.example.com:8443/")
        assert rest == "https://gateway.example.com:8443"
        assert flight == "grpc+tls://gateway.example.com:8443"

    def test_strips_whitespace(self) -> None:
        rest, flight = _build_urls("  gateway.example.com  ")
        assert rest == "https://gateway.example.com"
        assert flight == "grpc+tls://gateway.example.com"

    def test_empty_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="must not be empty"):
            _build_urls("")

    def test_whitespace_only_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="must not be empty"):
            _build_urls("   ")

    def test_port_without_host_raises(self) -> None:
        with pytest.raises(DCHConfigError, match="unable to extract host"):
            _build_urls(":8443")

    def test_localhost(self) -> None:
        rest, flight = _build_urls("localhost")
        assert rest == "https://localhost"
        assert flight == "grpc+tls://localhost"
