"""REST auth tests: verify HTTP auth middleware rejects/accepts correctly."""

from __future__ import annotations

import pytest

from data_connect_hub import (
    DataConnectClient,
    DCHAuthenticationError,
    DCHForbiddenError,
)


def _make_client(gateway_endpoint: str, *, token: str, tenant_id: str, insecure: bool) -> DataConnectClient:
    return DataConnectClient(
        gateway_endpoint,
        token=token,
        tenant_id=tenant_id,
        insecure=insecure,
        max_retries=0,
        rest_timeout=10.0,
    )


class TestRestAuth:
    def test_no_token_returns_401(self, gateway_endpoint: str, tenant_id: str, insecure: bool) -> None:
        client = _make_client(gateway_endpoint, token="", tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHAuthenticationError):
            client.list_connections()

    def test_invalid_token_returns_401(self, gateway_endpoint: str, tenant_id: str, insecure: bool) -> None:
        client = _make_client(gateway_endpoint, token="invalid-token", tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHAuthenticationError):
            client.list_connections()

    def test_denied_user_returns_403(
        self, gateway_endpoint: str, persona_denied_auth_token: str, tenant_id: str, insecure: bool
    ) -> None:
        if not persona_denied_auth_token:
            pytest.skip("DCH_PERSONA_DENIED_AUTH_TOKEN not set")
        client = _make_client(gateway_endpoint, token=persona_denied_auth_token, tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHForbiddenError):
            client.list_connections()

    def test_valid_auth_returns_200(self, rest_client: DataConnectClient) -> None:
        connections = rest_client.list_connections()
        assert isinstance(connections, list)
