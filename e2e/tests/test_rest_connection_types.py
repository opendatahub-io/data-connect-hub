"""Connection type CRUD via REST API."""

from __future__ import annotations

from data_connect_hub import DataConnectClient


class TestRestConnectionType:
    def test_crud(self, rest_client: DataConnectClient, create_connection_type) -> None:
        ct = create_connection_type(name="e2e-postgres-type", provider="postgres")

        types = rest_client.list_connection_types()
        assert ct.id in [t.id for t in types]

        fetched = rest_client.get_connection_type(ct.id)
        assert fetched.name == "e2e-postgres-type"
        assert fetched.provider == "postgres"
