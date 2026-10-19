"""REST check-connection and test-credentials endpoints."""

from __future__ import annotations

import os
import uuid
from pathlib import Path

import pytest
from data_connect_hub import DataConnectClient, DCHHTTPError


class TestRestCheckConnection:
    def test_readiness_success(
        self,
        rest_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        rest_client.check_connection_readiness(pg_flight_connection)

    def test_readiness_updates_status_to_ready(
        self,
        rest_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        rest_client.check_connection_readiness(pg_flight_connection)
        conn = rest_client.get_connection(pg_flight_connection)
        assert conn.status.state == "ready"

    def test_readiness_nonexistent_connection(
        self,
        rest_client: DataConnectClient,
    ) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHHTTPError):
            rest_client.check_connection_readiness(fake_id)


class TestRestTestCredentials:
    def test_valid_credentials(
        self,
        rest_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        pg_url = os.environ.get("DCH_TENANT_PG_URL")
        if not pg_url:
            pytest.skip("DCH_TENANT_PG_URL not set (raw PG URL needed)")

        conn = rest_client.get_connection(pg_flight_connection)
        dct_id = conn.data_connection_type_id

        secret: dict[str, str] = {"URI": pg_url}
        ca_cert_path = os.environ.get("DCH_TENANT_PG_CA_CERT")
        if ca_cert_path and os.path.exists(ca_cert_path):
            secret["CA_CERT"] = Path(ca_cert_path).read_text()

        rest_client.test_credentials(dct_id, secret)

    def test_s3_empty_region(
        self,
        rest_client: DataConnectClient,
        s3_flight_connection: str,
    ) -> None:
        """Regression: empty region must not cause 'failed to create S3 operator'."""
        access_key = os.environ.get("AWS_ACCESS_KEY_ID")
        secret_key = os.environ.get("AWS_SECRET_ACCESS_KEY")
        bucket = os.environ.get("AWS_S3_BUCKET")
        endpoint = os.environ.get("AWS_S3_ENDPOINT")
        if not all([access_key, secret_key, bucket, endpoint]):
            pytest.skip("AWS S3 env vars not set")

        conn = rest_client.get_connection(s3_flight_connection)
        dct_id = conn.data_connection_type_id

        secret: dict[str, str] = {
            "AWS_ACCESS_KEY_ID": access_key,
            "AWS_SECRET_ACCESS_KEY": secret_key,
            "AWS_S3_BUCKET": bucket,
            "AWS_S3_ENDPOINT": endpoint,
            "AWS_DEFAULT_REGION": "",
        }
        ca_cert_path = os.environ.get("AWS_S3_CA_CERT")
        if ca_cert_path and os.path.exists(ca_cert_path):
            secret["AWS_S3_CA_CERT"] = Path(ca_cert_path).read_text()

        rest_client.test_credentials(dct_id, secret)

    def test_invalid_credentials(
        self,
        rest_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        conn = rest_client.get_connection(pg_flight_connection)
        dct_id = conn.data_connection_type_id

        with pytest.raises(DCHHTTPError) as exc_info:
            rest_client.test_credentials(
                dct_id,
                {"URI": "postgresql://invalid:invalid@nonexistent:5432/nope"},
            )
        assert exc_info.value.status_code == 502

    def test_nonexistent_connection_type(
        self,
        rest_client: DataConnectClient,
    ) -> None:
        fake_type_id = str(uuid.uuid4())
        with pytest.raises(DCHHTTPError):
            rest_client.test_credentials(
                fake_type_id,
                {"URI": "postgresql://x:x@localhost:5432/x"},
            )
