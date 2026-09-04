"""REST binary download tests: download raw files via the REST binary endpoint.

Hits ``GET /api/v1alpha1/data/connections/{id}/binary?path=<path>`` and
verifies correct streaming, headers, and error handling.

Requires S3 or URI credentials in the env file.
Skips automatically if neither is configured.
"""

from __future__ import annotations

import httpx
import pytest

API_PREFIX = "/api/v1alpha1/data"


def _download_binary(
    http_client: httpx.Client,
    connection_id: str,
    path: str,
    tenant_id: str,
    auth_token: str,
) -> httpx.Response:
    return http_client.get(
        f"{API_PREFIX}/connections/{connection_id}/binary",
        params={"path": path},
        headers={
            "x-tenant-id": tenant_id,
            "authorization": f"Bearer {auth_token}",
        },
    )


class TestRestS3Binary:
    def test_binary_download(
        self,
        http_client: httpx.Client,
        s3_flight_connection: str,
        s3_binary_path: str | None,
        tenant_id: str,
        auth_token: str,
    ) -> None:
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set")

        resp = _download_binary(http_client, s3_flight_connection, s3_binary_path, tenant_id, auth_token)

        assert resp.status_code == 200
        assert resp.headers["content-type"] == "application/octet-stream"
        assert "content-disposition" in resp.headers
        assert resp.content == b"binary-test-data-for-e2e\n"

    def test_binary_download_not_found(
        self,
        http_client: httpx.Client,
        s3_flight_connection: str,
        s3_binary_path: str | None,
        tenant_id: str,
        auth_token: str,
    ) -> None:
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set (need S3 configured)")

        resp = _download_binary(http_client, s3_flight_connection, "nonexistent/path/file.bin", tenant_id, auth_token)

        assert resp.status_code == 404

    def test_binary_download_missing_path_param(
        self,
        http_client: httpx.Client,
        s3_flight_connection: str,
        s3_binary_path: str | None,
        tenant_id: str,
        auth_token: str,
    ) -> None:
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set (need S3 configured)")

        resp = http_client.get(
            f"{API_PREFIX}/connections/{s3_flight_connection}/binary",
            headers={
                "x-tenant-id": tenant_id,
                "authorization": f"Bearer {auth_token}",
            },
        )

        assert resp.status_code == 400


class TestRestUriBinary:
    def test_binary_download(
        self,
        http_client: httpx.Client,
        uri_flight_connection: str,
        tenant_id: str,
        auth_token: str,
    ) -> None:
        resp = _download_binary(http_client, uri_flight_connection, "api/binary.dat", tenant_id, auth_token)

        assert resp.status_code == 200
        assert resp.headers["content-type"] == "application/octet-stream"
        assert "content-disposition" in resp.headers
        assert resp.content == b"binary-test-data-for-e2e\n"

    def test_binary_download_not_found(
        self,
        http_client: httpx.Client,
        uri_flight_connection: str,
        tenant_id: str,
        auth_token: str,
    ) -> None:
        resp = _download_binary(http_client, uri_flight_connection, "nonexistent/path.bin", tenant_id, auth_token)

        assert resp.status_code == 404


class TestRestBinaryUnsupported:
    def test_unsupported_connector_returns_501(
        self,
        http_client: httpx.Client,
        pg_flight_connection: str,
        tenant_id: str,
        auth_token: str,
    ) -> None:
        """Postgres does not support binary reads."""
        resp = _download_binary(http_client, pg_flight_connection, "some/path", tenant_id, auth_token)

        assert resp.status_code == 501
        body = resp.json()
        assert body["code"] == "unsupported_operation"
