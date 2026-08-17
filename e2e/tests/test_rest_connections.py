"""Connection CRUD via REST API."""

from __future__ import annotations

import pytest

from data_connect_hub import AdminSecretRef, DataConnectClient


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
        assert fetched.properties["database"] == "testdb"
