"""Verify CRUD remains available when a connector is disabled."""

from __future__ import annotations

import json
import os

import pytest
from data_connect_hub import CredentialsRef, DataConnectClient, DCHQueryError, DCHServerError


class TestDisabledConnector:
    def test_crud_succeeds_but_ingestion_fails(
        self,
        dch_client: DataConnectClient,
        rest_client: DataConnectClient,
        create_connection_type,
        create_connection,
        uri_secret: str | None,
    ) -> None:
        disabled_connector = os.getenv("E2E_DISABLED_CONNECTOR")
        if not disabled_connector:
            pytest.skip("E2E_DISABLED_CONNECTOR is empty")
        if not uri_secret:
            pytest.skip("DCH_URI_SECRET not set")

        connection_type = create_connection_type(
            name=f"e2e-disabled-{disabled_connector}-type",
            provider=disabled_connector,
            description="disabled connector test",
        )

        fetched_type = rest_client.get_connection_type(connection_type.id)
        assert fetched_type.provider == disabled_connector

        updated_type = rest_client.update_connection_type(
            connection_type.id,
            description="updated disabled connector test",
        )
        assert updated_type.description == "updated disabled connector test"

        connection = create_connection(
            name=f"e2e-disabled-{disabled_connector}-connection",
            connection_type_id=connection_type.id,
            credentials_ref=CredentialsRef(secret=uri_secret),
        )

        fetched_connection = rest_client.get_connection(connection.id)
        assert fetched_connection.data_connection_type_id == connection_type.id

        updated_connection = rest_client.update_connection(
            connection.id,
            name=f"e2e-updated-disabled-{disabled_connector}-connection",
        )
        assert updated_connection.name == f"e2e-updated-disabled-{disabled_connector}-connection"

        connector_error = f"no connector registered for provider '{disabled_connector}'"
        with pytest.raises(DCHQueryError) as tabular_error:
            dch_client.read(json.dumps({"path": "/api/cities.json"}), connection.id)
        assert connector_error in str(tabular_error.value)

        with pytest.raises(DCHServerError) as binary_error:
            b"".join(rest_client.download_binary(connection.id, "api/binary.dat"))
        assert binary_error.value.status_code == 500
        error_body = json.loads(binary_error.value.body)
        assert error_body["code"] == "flight_service_error"
        assert error_body["message"] == f"connector configuration error: {connector_error}"
