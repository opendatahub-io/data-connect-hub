"""Export connection credentials to a K8s secret via REST API."""

from __future__ import annotations

import uuid

import pytest
from data_connect_hub import DataConnectClient, DCHForbiddenError, DCHHTTPError


class TestRestExportConnection:
    def test_export_requires_secret_read(
        self,
        rest_client: DataConnectClient,
        secret_reader_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        # The e2e user passes the kube-rbac-proxy check (dch-secret-export)
        # but cannot read the connection's credentials_ref secret, so the
        # service-side SubjectAccessReview must deny the export with 403.
        # The secret-reader SA holds the same API access plus the secret
        # read, so exporting the same secret succeeds.
        secret_name = f"e2e-export-{uuid.uuid4().hex[:8]}"
        with pytest.raises(DCHForbiddenError) as exc_info:
            rest_client.export_connection(pg_flight_connection, secret_name)
        assert exc_info.value.status_code == 403
        secret_reader_client.export_connection(pg_flight_connection, secret_name)

    def test_export_success(
        self,
        secret_reader_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        secret_name = f"e2e-export-{uuid.uuid4().hex[:8]}"
        secret_reader_client.export_connection(pg_flight_connection, secret_name)

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
        secret_reader_client: DataConnectClient,
        pg_flight_connection: str,
    ) -> None:
        secret_name = f"e2e-export-{uuid.uuid4().hex[:8]}"
        secret_reader_client.export_connection(pg_flight_connection, secret_name)
        secret_reader_client.export_connection(pg_flight_connection, secret_name)
