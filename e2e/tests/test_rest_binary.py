"""SDK binary download tests.

Requires S3 or URI credentials in the env file.
Skips automatically if neither is configured.
"""

from __future__ import annotations

import pytest
from data_connect_hub import DCHConfigError, DCHHTTPError, DataConnectClient

class TestRestS3Binary:
    def test_binary_download(
        self,
        rest_client: DataConnectClient,
        s3_flight_connection: str,
        s3_binary_path: str | None,
    ) -> None:
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set")

        assert rest_client.download_binary(s3_flight_connection, s3_binary_path) == b"binary-test-data-for-e2e\n"

    def test_binary_download_not_found(
        self,
        rest_client: DataConnectClient,
        s3_flight_connection: str,
        s3_binary_path: str | None,
    ) -> None:
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set (need S3 configured)")

        with pytest.raises(DCHHTTPError) as exc_info:
            rest_client.download_binary(s3_flight_connection, "nonexistent/path/file.bin")
        assert exc_info.value.status_code == 404

    def test_binary_download_missing_path_param(
        self,
        rest_client: DataConnectClient,
        s3_flight_connection: str,
        s3_binary_path: str | None,
    ) -> None:
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set (need S3 configured)")

        with pytest.raises(DCHConfigError, match="path must be"):
            rest_client.download_binary(s3_flight_connection, "")


class TestRestUriBinary:
    def test_binary_download(
        self,
        rest_client: DataConnectClient,
        uri_flight_connection: str,
    ) -> None:
        assert rest_client.download_binary(uri_flight_connection, "api/binary.dat") == b"binary-test-data-for-e2e\n"

    def test_binary_download_not_found(
        self,
        rest_client: DataConnectClient,
        uri_flight_connection: str,
    ) -> None:
        with pytest.raises(DCHHTTPError) as exc_info:
            rest_client.download_binary(uri_flight_connection, "nonexistent/path.bin")
        assert exc_info.value.status_code == 404


class TestRestBinaryUnsupported:
    def test_unsupported_connector_returns_501(
        self,
        rest_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        """Postgres does not support binary reads."""
        with pytest.raises(DCHHTTPError) as exc_info:
            rest_client.download_binary(pg_flight_connection, "some/path")
        assert exc_info.value.status_code == 501
