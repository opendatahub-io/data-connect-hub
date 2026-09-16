"""Connection type CRUD via REST API."""

from __future__ import annotations

import time
import uuid

import pytest
from data_connect_hub import DataConnectClient, DCHNotFoundError

EXPECTED_OOB_CONNECTION_TYPES = {
    "Postgres": "postgres",
    "PGVector": "postgres",
    "ElasticSearch": "elasticsearch",
    "HuggingFace": "huggingface",
    "Milvus": "milvus",
    "Neo4j": "neo4j",
    "URI": "uri",
}


class TestRestConnectionType:
    def test_oob_connection_types_are_registered(self, rest_client: DataConnectClient) -> None:
        """Verify the controller registers every built-in connection type."""
        deadline = time.monotonic() + 30
        listed_types = []
        while time.monotonic() < deadline:
            listed_types = rest_client.list_connection_types()
            if all(
                sum(connection_type.name == name for connection_type in listed_types) == 1
                for name in EXPECTED_OOB_CONNECTION_TYPES
            ):
                break
            time.sleep(1)

        for name, provider in EXPECTED_OOB_CONNECTION_TYPES.items():
            matching_types = [connection_type for connection_type in listed_types if connection_type.name == name]
            assert len(matching_types) == 1, f"expected exactly one OOB connection type named {name!r}"
            assert matching_types[0].provider == provider

        by_name = {connection_type.name: connection_type for connection_type in listed_types}
        postgres_schema = {(field.name, field.required, field.type) for field in by_name["Postgres"].credentials_fields}
        pgvector_schema = {(field.name, field.required, field.type) for field in by_name["PGVector"].credentials_fields}
        assert pgvector_schema == postgres_schema

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

    def test_update(self, rest_client: DataConnectClient, create_connection_type) -> None:
        ct = create_connection_type(provider="postgres")
        updated = rest_client.update_connection_type(ct.id, description="updated description")
        assert updated.description == "updated description"
