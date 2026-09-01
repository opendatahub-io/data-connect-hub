"""Smoke tests: verify services are reachable."""

from __future__ import annotations

import httpx


class TestRestHealth:
    def test_health_endpoint(self, http_client: httpx.Client, gateway_auth_required: bool, auth_token: str) -> None:
        headers = {"Authorization": f"Bearer {auth_token}"} if gateway_auth_required else None
        resp = http_client.get("/health", headers=headers)
        assert resp.status_code == 200
        body = resp.json()
        assert "service" in body
