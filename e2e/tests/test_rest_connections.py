"""Connection CRUD via REST API."""

from __future__ import annotations

import uuid

import pytest

from data_connect_hub import AdminSecretRef, DataConnectClient, DCHNotFoundError


@pytest.fixture()
def pg_connection_type(create_connection_type):
    return create_connection_type(provider="postgres")


class TestRestConnection:
    def test_crud(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
    ) -> None:
        conn = create_connection(
            name="e2e-my-pg",
            connection_type_id=pg_connection_type.id,
            admin=AdminSecretRef(secret_ref="e2e-ns/e2e-secret"),
            properties={"database": "testdb"},
        )

        connections = rest_client.list_connections()
        assert conn.id in [c.id for c in connections]

        fetched = rest_client.get_connection(conn.id)
        assert fetched.name == "e2e-my-pg"
        assert fetched.data_connection_type_id == pg_connection_type.id
        assert fetched.properties["database"] == "testdb"

    def test_delete(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
    ) -> None:
        conn = create_connection(
            name="e2e-delete-pg",
            connection_type_id=pg_connection_type.id,
            admin=AdminSecretRef(secret_ref="e2e-ns/e2e-secret"),
        )
        rest_client.delete_connection(conn.id)

        with pytest.raises(DCHNotFoundError):
            rest_client.get_connection(conn.id)

    def test_get_nonexistent_returns_404(self, rest_client: DataConnectClient) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHNotFoundError):
            rest_client.get_connection(fake_id)

    def test_delete_nonexistent_returns_404(self, rest_client: DataConnectClient) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHNotFoundError):
            rest_client.delete_connection(fake_id)

    @pytest.mark.skip(reason="PATCH /connections/{id} not implemented (server returns 501)")
    def test_update(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
    ) -> None:
        conn = create_connection(
            name="e2e-update-pg",
            connection_type_id=pg_connection_type.id,
            admin=AdminSecretRef(secret_ref="e2e-ns/e2e-secret"),
            properties={"database": "testdb"},
        )
        updated = rest_client.update_connection(conn.id, name="e2e-updated-pg")
        assert updated.name == "e2e-updated-pg"
