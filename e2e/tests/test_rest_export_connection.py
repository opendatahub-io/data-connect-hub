"""Export connection credentials to a K8s secret via REST API."""

from __future__ import annotations

import uuid

import pytest
from data_connect_hub import DataConnectClient, DCHHTTPError


class TestRestExportConnection:
    def test_export_success(
        self,
        rest_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        secret_name = f"e2e-export-{uuid.uuid4().hex[:8]}"
        rest_client.export_connection(pg_flight_connection, secret_name)

    def test_export_nonexistent_connection_returns_404(
        self,
        rest_client: DataConnectClient,
    ) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHHTTPError) as exc_info:
            rest_client.export_connection(fake_id, f"e2e-export-{uuid.uuid4().hex[:8]}")
        assert exc_info.value.status_code == 404

    def test_export_idempotent(
        self,
        rest_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        secret_name = f"e2e-export-{uuid.uuid4().hex[:8]}"
        rest_client.export_connection(pg_flight_connection, secret_name)
        rest_client.export_connection(pg_flight_connection, secret_name)
