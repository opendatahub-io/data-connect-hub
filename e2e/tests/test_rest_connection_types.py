"""Connection type CRUD via REST API."""

from __future__ import annotations

import time
import uuid

import pytest
from data_connect_hub import CredentialField, DataConnectClient, DCHNotFoundError

EXPECTED_OOB_CONNECTION_TYPES = {
    "Postgres": "postgres",
    "PGVector": "postgres",
    "ElasticSearch": "elasticsearch",
    "HuggingFace": "huggingface",
    "Milvus": "milvus",
    "Neo4j": "neo4j",
    "URI": "uri",
}

EXPECTED_SUPPORTED_FORMATS = {
    "Postgres": ["tabular"],
    "PGVector": ["tabular"],
    "ElasticSearch": ["tabular"],
    "HuggingFace": [],
    "Milvus": ["tabular"],
    "Neo4j": ["tabular"],
    "URI": ["tabular", "binary"],
}

EXPECTED_CREDENTIAL_FIELDS = {
    "ElasticSearch": {"URI", "USERNAME", "PASSWORD", "CA_CERT", "API_KEY"},
    "HuggingFace": {"URI", "TOKEN"},
    "Milvus": {"URI", "TOKEN", "DATABASE", "CA_CERT"},
    "Neo4j": {"URI", "USERNAME", "PASSWORD", "DATABASE", "CA_CERT"},
}

# Credential fields that must be marked sensitive in every OOB type that has them.
# URI is included because connection URIs may embed passwords.
SENSITIVE_CREDENTIAL_FIELDS = {"URI", "PASSWORD", "TOKEN", "API_KEY"}


class TestRestConnectionType:
    def test_oob_connection_types(self, rest_client: DataConnectClient) -> None:
        """Verify all OOB connection types are registered with correct attributes."""
        deadline = time.monotonic() + 30
        listed_types = []
        while time.monotonic() < deadline:
            listed_types = rest_client.list_connection_types()
            if all(
                sum(ct.name == name for ct in listed_types) == 1
                for name in EXPECTED_OOB_CONNECTION_TYPES
            ):
                break
            time.sleep(1)

        by_name = {ct.name: ct for ct in listed_types}

        # 1. every expected type exists with the correct provider
        for name, provider in EXPECTED_OOB_CONNECTION_TYPES.items():
            ct = by_name.get(name)
            assert ct is not None, f"expected OOB connection type {name!r} not found"
            assert ct.provider == provider

        # 2. credential field schemas
        for name, expected_fields in EXPECTED_CREDENTIAL_FIELDS.items():
            actual_fields = {field.name for field in by_name[name].credentials_fields}
            assert actual_fields == expected_fields

        postgres_schema = {(f.name, f.required, f.type) for f in by_name["Postgres"].credentials_fields}
        pgvector_schema = {(f.name, f.required, f.type) for f in by_name["PGVector"].credentials_fields}
        assert pgvector_schema == postgres_schema

        # 2b. sensitive flags on credential fields
        for name in EXPECTED_OOB_CONNECTION_TYPES:
            for field in by_name[name].credentials_fields:
                expected = field.name in SENSITIVE_CREDENTIAL_FIELDS
                assert field.sensitive is expected, f"{name}.{field.name}: sensitive={field.sensitive}, want {expected}"

        # 3. supported_formats set correctly after flight registration
        for name, expected_formats in EXPECTED_SUPPORTED_FORMATS.items():
            ct = by_name[name]
            assert sorted(ct.status.supported_formats) == sorted(expected_formats), (
                f"{name}: expected supported_formats={expected_formats}, "
                f"got {ct.status.supported_formats}"
            )

    def test_crud(self, rest_client: DataConnectClient, create_connection_type) -> None:
        ct = create_connection_type(
            name="e2e-postgres-type",
            provider="postgres",
            description="e2e crud test type",
            credentials_fields=[
                CredentialField(name="URI", label="URI", required=True, type="string", sensitive=True),
                CredentialField(name="CA_CERT", label="CA Certificate", required=False, type="string"),
            ],
        )

        types = rest_client.list_connection_types()
        assert ct.id in [t.id for t in types]

        fetched = rest_client.get_connection_type(ct.id)
        assert fetched.name == "e2e-postgres-type"
        assert fetched.provider == "postgres"
        assert fetched.description == "e2e crud test type"
        sensitive = {f.name: f.sensitive for f in fetched.credentials_fields}
        assert sensitive == {"URI": True, "CA_CERT": False}

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

    def test_update_sensitive(self, rest_client: DataConnectClient, create_connection_type) -> None:
        ct = create_connection_type(
            provider="postgres",
            credentials_fields=[CredentialField(name="TOKEN", label="Token", required=False, type="string")],
        )
        assert ct.credentials_fields[0].sensitive is False

        rest_client.update_connection_type(
            ct.id,
            credentials_fields=[
                CredentialField(name="TOKEN", label="Token", required=False, type="string", sensitive=True)
            ],
        )
        fetched = rest_client.get_connection_type(ct.id)
        assert fetched.credentials_fields[0].sensitive is True
