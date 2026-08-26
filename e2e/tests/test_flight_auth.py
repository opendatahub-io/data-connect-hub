"""Flight auth tests: verify gRPC auth middleware rejects/accepts correctly."""

from __future__ import annotations

import pytest

from data_connect_hub import DataConnectClient, DCHConnectionError


class TestFlightAuth:
    def test_no_token_is_rejected(self, gateway_endpoint: str, tenant_id: str, insecure: bool) -> None:
        client = DataConnectClient(gateway_endpoint, token="", tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHConnectionError, match="(?i)unauthenticated"):
            client.server_info()

    def test_invalid_token_is_rejected(self, gateway_endpoint: str, tenant_id: str, insecure: bool) -> None:
        client = DataConnectClient(gateway_endpoint, token="invalid-token", tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHConnectionError, match="(?i)unauthenticated"):
            client.server_info()

    def test_missing_tenant_id_is_rejected(self, gateway_endpoint: str, auth_token: str, insecure: bool) -> None:
        client = DataConnectClient(gateway_endpoint, token=auth_token, tenant_id="", insecure=insecure)
        with pytest.raises(DCHConnectionError, match="(?i)unauthorized|permission.?denied"):
            client.server_info()

    def test_denied_user_is_rejected(
        self, gateway_endpoint: str, denied_auth_token: str, tenant_id: str, insecure: bool
    ) -> None:
        if not denied_auth_token:
            pytest.skip("DCH_DENIED_AUTH_TOKEN not set")
        client = DataConnectClient(gateway_endpoint, token=denied_auth_token, tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHConnectionError, match="(?i)unauthorized|permission.?denied"):
            client.server_info()

    def test_wrong_namespace_is_rejected(
        self, gateway_endpoint: str, auth_token: str, no_access_namespace: str, insecure: bool
    ) -> None:
        client = DataConnectClient(gateway_endpoint, token=auth_token, tenant_id=no_access_namespace, insecure=insecure)
        with pytest.raises(DCHConnectionError, match="(?i)unauthorized|permission.?denied"):
            client.server_info()

    def test_valid_auth_succeeds(self, gateway_endpoint: str, auth_token: str, tenant_id: str, insecure: bool) -> None:
        client = DataConnectClient(gateway_endpoint, token=auth_token, tenant_id=tenant_id, insecure=insecure)
        info = client.server_info()
        assert isinstance(info, dict)
