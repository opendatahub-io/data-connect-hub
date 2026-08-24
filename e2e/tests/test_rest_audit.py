"""Audit endpoint RBAC tests.

POST /api-internal/v1/audit/data-connection-types is internal-only: it must
be reachable by dc-controller's ServiceAccount and no one else, regardless
of tenant RBAC.
"""

from __future__ import annotations

import httpx
import pytest

AUDIT_PATH = "/api-internal/v1/audit/data-connection-types"


class TestAuditEndpointAuth:
    def test_no_token_returns_401(self, http_client: httpx.Client) -> None:
        resp = http_client.post(AUDIT_PATH)
        assert resp.status_code == 401

    def test_invalid_token_returns_401(self, http_client: httpx.Client) -> None:
        resp = http_client.post(AUDIT_PATH, headers={"Authorization": "Bearer invalid-token"})
        assert resp.status_code == 401

    def test_regular_tenant_user_returns_403(self, http_client: httpx.Client, auth_token: str) -> None:
        resp = http_client.post(AUDIT_PATH, headers={"Authorization": f"Bearer {auth_token}"})
        assert resp.status_code == 403

    def test_denied_user_returns_403(self, http_client: httpx.Client, denied_auth_token: str) -> None:
        if not denied_auth_token:
            pytest.skip("DCH_DENIED_AUTH_TOKEN not set")
        resp = http_client.post(AUDIT_PATH, headers={"Authorization": f"Bearer {denied_auth_token}"})
        assert resp.status_code == 403

    def test_audit_invoker_returns_202(self, http_client: httpx.Client, audit_invoker_token: str) -> None:
        if not audit_invoker_token:
            pytest.skip("DCH_AUDIT_INVOKER_TOKEN not set")
        resp = http_client.post(AUDIT_PATH, headers={"Authorization": f"Bearer {audit_invoker_token}"})
        assert resp.status_code == 202
