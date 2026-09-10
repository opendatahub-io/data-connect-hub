"""Audit data-connection-types endpoint via REST API."""

from __future__ import annotations

import httpx
import pytest

from data_connect_hub import DataConnectClient


AUDIT_PATH = "/api/v1alpha1/audit/data-connection-types"


class TestRestAuditConnectionTypes:
    """POST /api/v1alpha1/audit/data-connection-types"""

    def test_returns_202(self, http_client: httpx.Client, auth_token: str) -> None:
        resp = http_client.post(
            AUDIT_PATH,
            headers={"Authorization": f"Bearer {auth_token}"},
        )
        assert resp.status_code == 202

    def test_no_token_returns_401(self, http_client: httpx.Client) -> None:
        resp = http_client.post(AUDIT_PATH)
        assert resp.status_code == 401

    def test_invalid_token_returns_401(self, http_client: httpx.Client) -> None:
        resp = http_client.post(
            AUDIT_PATH,
            headers={"Authorization": "Bearer invalid-token"},
        )
        assert resp.status_code == 401

    def test_denied_token_returns_403(
        self, http_client: httpx.Client, denied_auth_token: str
    ) -> None:
        if not denied_auth_token:
            pytest.skip("DCH_DENIED_AUTH_TOKEN not set")
        resp = http_client.post(
            AUDIT_PATH,
            headers={"Authorization": f"Bearer {denied_auth_token}"},
        )
        assert resp.status_code == 403

    def test_does_not_require_tenant_header(
        self, http_client: httpx.Client, auth_token: str
    ) -> None:
        resp = http_client.post(
            AUDIT_PATH,
            headers={"Authorization": f"Bearer {auth_token}"},
        )
        assert resp.status_code == 202
        assert "x-tenant-id" not in {k.lower() for k in resp.request.headers}

    def test_get_method_returns_403(self, http_client: httpx.Client, auth_token: str) -> None:
        resp = http_client.get(
            AUDIT_PATH,
            headers={"Authorization": f"Bearer {auth_token}"},
        )
        assert resp.status_code == 403

    def test_updates_connection_type_capabilities(
        self,
        http_client: httpx.Client,
        auth_token: str,
        rest_client: DataConnectClient,
        create_connection_type,
    ) -> None:
        ct = create_connection_type(provider="postgres")

        resp = http_client.post(
            AUDIT_PATH,
            headers={"Authorization": f"Bearer {auth_token}"},
        )
        assert resp.status_code == 202

        fetched = rest_client.get_connection_type(ct.id)
        assert fetched.status.capabilities.flight is True

    def test_unknown_provider_gets_flight_false(
        self,
        http_client: httpx.Client,
        auth_token: str,
        rest_client: DataConnectClient,
        create_connection_type,
    ) -> None:
        ct = create_connection_type(
            provider="nonexistent-provider-e2e",
            description="e2e audit test – unknown provider",
        )

        resp = http_client.post(
            AUDIT_PATH,
            headers={"Authorization": f"Bearer {auth_token}"},
        )
        assert resp.status_code == 202

        fetched = rest_client.get_connection_type(ct.id)
        assert fetched.status.capabilities.flight is False

    def test_idempotent(
        self, http_client: httpx.Client, auth_token: str
    ) -> None:
        headers = {"Authorization": f"Bearer {auth_token}"}
        resp1 = http_client.post(AUDIT_PATH, headers=headers)
        resp2 = http_client.post(AUDIT_PATH, headers=headers)
        assert resp1.status_code == 202
        assert resp2.status_code == 202
