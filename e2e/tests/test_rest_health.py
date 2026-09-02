"""Smoke tests: verify services are reachable."""

from __future__ import annotations

import httpx


class TestRestHealth:
    def test_health_endpoint(self, http_client: httpx.Client) -> None:
        resp = http_client.get("/health")
        assert resp.status_code == 200
        body = resp.json()
        assert "service" in body
