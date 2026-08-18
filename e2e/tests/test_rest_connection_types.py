"""Connection type CRUD via REST API."""

from __future__ import annotations

import uuid

import pytest

from data_connect_hub import DataConnectClient, DCHNotFoundError


class TestRestConnectionType:
    def test_crud(self, rest_client: DataConnectClient, create_connection_type) -> None:
        ct = create_connection_type(
            name="e2e-postgres-type",
            provider="postgres",
            description="e2e crud test type",
        )

        types = rest_client.list_connection_types()
        assert ct.id in [t.id for t in types]

        fetched = rest_client.get_connection_type(ct.id)
        assert fetched.name == "e2e-postgres-type"
        assert fetched.provider == "postgres"
        assert fetched.description == "e2e crud test type"

    def test_delete(self, rest_client: DataConnectClient, create_connection_type) -> None:
        ct = create_connection_type(provider="postgres")
        rest_client.delete_connection_type(ct.id)

        with pytest.raises(DCHNotFoundError):
            rest_client.get_connection_type(ct.id)

    def test_get_nonexistent_returns_404(self, rest_client: DataConnectClient) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHNotFoundError):
            rest_client.get_connection_type(fake_id)

    def test_delete_nonexistent_returns_404(self, rest_client: DataConnectClient) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHNotFoundError):
            rest_client.delete_connection_type(fake_id)

    @pytest.mark.skip(reason="PATCH /connection-types/{id} not implemented (server returns 501)")
    def test_update(self, rest_client: DataConnectClient, create_connection_type) -> None:
        ct = create_connection_type(provider="postgres")
        updated = rest_client.update_connection_type(ct.id, description="updated description")
        assert updated.description == "updated description"
