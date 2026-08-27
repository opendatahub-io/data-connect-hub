"""Pre-creation credential check via POST /api/v1/data/test/credentials."""

from __future__ import annotations

import uuid

import httpx
import pytest


@pytest.fixture()
def pg_connection_type(create_connection_type):
    return create_connection_type(provider="postgres")


BAD_PG_URI = "postgresql://e2e:wrong-password@127.0.0.1:1/nope"


class TestRestTestCredentials:
    def test_valid_credentials(
        self,
        authed_http_client: httpx.Client,
        pg_url: str | None,
        pg_connection_type,
    ) -> None:
        if not pg_url:
            pytest.skip("DCH_TENANT_PG_URL not set (run e2e/run-e2e.sh first)")
        resp = authed_http_client.post(
            "/api/v1/data/test/credentials",
            json={"data_connection_type_id": pg_connection_type.id, "secret": {"URI": pg_url}},
        )
        assert resp.status_code == 204

    def test_invalid_credentials(self, authed_http_client: httpx.Client, pg_connection_type) -> None:
        resp = authed_http_client.post(
            "/api/v1/data/test/credentials",
            json={"data_connection_type_id": pg_connection_type.id, "secret": {"URI": BAD_PG_URI}},
        )
        assert resp.status_code == 502

    def test_missing_required_field(self, authed_http_client: httpx.Client, pg_connection_type) -> None:
        resp = authed_http_client.post(
            "/api/v1/data/test/credentials",
            json={"data_connection_type_id": pg_connection_type.id, "secret": {}},
        )
        assert resp.status_code == 502

    def test_nonexistent_connection_type(self, authed_http_client: httpx.Client) -> None:
        resp = authed_http_client.post(
            "/api/v1/data/test/credentials",
            json={"data_connection_type_id": str(uuid.uuid4()), "secret": {"URI": BAD_PG_URI}},
        )
        assert resp.status_code == 502
