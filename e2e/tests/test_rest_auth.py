"""REST auth tests: verify HTTP auth middleware rejects/accepts correctly."""

from __future__ import annotations

import pytest

from data_connect_hub import (
    DCHAuthenticationError,
    DCHForbiddenError,
    DCHHTTPError,
    DataConnectClient,
)


def _make_client(rest_url: str, *, token: str, tenant_id: str, insecure: bool) -> DataConnectClient:
    return DataConnectClient(
        rest_url=rest_url,
        token=token,
        tenant_id=tenant_id,
        insecure=insecure,
        max_retries=0,
        rest_timeout=10.0,
    )


class TestRestAuth:
    def test_no_token_returns_401(self, rest_url: str, tenant_id: str, insecure: bool) -> None:
        client = _make_client(rest_url, token="", tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHAuthenticationError):
            client.list_connections()

    def test_invalid_token_returns_401(self, rest_url: str, tenant_id: str, insecure: bool) -> None:
        client = _make_client(rest_url, token="invalid-token", tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHAuthenticationError):
            client.list_connections()

    def test_missing_tenant_returns_400(self, rest_url: str, auth_token: str, insecure: bool) -> None:
        client = _make_client(rest_url, token=auth_token, tenant_id="", insecure=insecure)
        with pytest.raises(DCHHTTPError) as exc_info:
            client.list_connections()
        assert exc_info.value.status_code == 400

    def test_denied_user_returns_403(
        self, rest_url: str, denied_auth_token: str, tenant_id: str, insecure: bool
    ) -> None:
        if not denied_auth_token:
            pytest.skip("DCH_DENIED_AUTH_TOKEN not set")
        client = _make_client(rest_url, token=denied_auth_token, tenant_id=tenant_id, insecure=insecure)
        with pytest.raises(DCHForbiddenError):
            client.list_connections()

    def test_wrong_namespace_returns_403(
        self, rest_url: str, auth_token: str, no_access_namespace: str, insecure: bool
    ) -> None:
        if not no_access_namespace:
            pytest.skip("DCH_NO_ACCESS_NAMESPACE not set")
        client = _make_client(rest_url, token=auth_token, tenant_id=no_access_namespace, insecure=insecure)
        with pytest.raises(DCHForbiddenError):
            client.list_connections()

    def test_valid_auth_returns_200(self, rest_client: DataConnectClient) -> None:
        connections = rest_client.list_connections()
        assert isinstance(connections, list)
