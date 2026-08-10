"""Tests for Pydantic data models."""

from __future__ import annotations

from datetime import UTC, datetime

from data_connect_hub.models import (
    AdminSecret,
    AdminSecretRef,
    ConnectionType,
    CreateConnectionRequest,
    DataConnection,
    EnumValue,
    UpdateConnectionRequest,
)

from .conftest import (
    SAMPLE_CONNECTION_JSON,
    SAMPLE_CONNECTION_TYPE_JSON,
    SAMPLE_CONNECTION_TYPE_WRAPPED_JSON,
    SAMPLE_CONNECTION_WRAPPED_JSON,
)


class TestDataConnection:
    def test_from_json_fixture(self) -> None:
        """Mirrors the Rust test in commons/src/api/connections.rs."""
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        assert conn.id == "123"
        assert conn.name == "test-conn"
        assert conn.data_connection_type_id == "postgres"
        assert conn.format == "tabular"
        assert conn.tenant_id == "tenant-1"
        assert conn.admin == AdminSecretRef(secret_ref="secret/test-conn")
        assert conn.created_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.updated_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.properties == {"key": "value"}

    def test_from_wrapped_json(self) -> None:
        """Server returns {metadata, resource} envelope."""
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_WRAPPED_JSON)
        assert conn.id == "123"
        assert conn.name == "test-conn"
        assert conn.data_connection_type_id == "postgres"
        assert conn.format == "tabular"
        assert conn.tenant_id == "tenant-1"
        assert conn.admin == AdminSecretRef(secret_ref="secret/test-conn")
        assert conn.created_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.updated_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.properties == {"key": "value"}

    def test_round_trip(self) -> None:
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        dumped = conn.model_dump()
        restored = DataConnection.model_validate(dumped)
        assert restored == conn

    def test_default_properties(self) -> None:
        data = dict(SAMPLE_CONNECTION_JSON)
        del data["properties"]
        conn = DataConnection.model_validate(data)
        assert conn.properties == {}


class TestAdmin:
    def test_secret_ref_round_trip(self) -> None:
        admin = AdminSecretRef(secret_ref="secret/my-conn")
        dumped = admin.model_dump()
        restored = AdminSecretRef.model_validate(dumped)
        assert restored == admin

    def test_secret_round_trip(self) -> None:
        admin = AdminSecret(secret={"username": "admin", "password": "s3cret"})
        dumped = admin.model_dump()
        restored = AdminSecret.model_validate(dumped)
        assert restored == admin
        assert restored.secret == {"username": "admin", "password": "s3cret"}

    def test_secret_variant_in_connection(self) -> None:
        data = dict(SAMPLE_CONNECTION_JSON)
        data["admin"] = {"secret": {"user": "root", "pass": "pw"}}
        conn = DataConnection.model_validate(data)
        assert isinstance(conn.admin, AdminSecret)
        assert conn.admin.secret == {"user": "root", "pass": "pw"}


class TestCreateConnectionRequest:
    def test_dump_excludes_none(self) -> None:
        req = CreateConnectionRequest(
            name="conn",
            data_connection_type_id="postgres",
            format="tabular",
        )
        dumped = req.model_dump(exclude_none=True)
        assert dumped["data_connection_type_id"] == "postgres"
        assert dumped["properties"] == {}
        assert "admin" not in dumped


class TestUpdateConnectionRequest:
    def test_partial_dump(self) -> None:
        req = UpdateConnectionRequest(name="new-name")
        dumped = req.model_dump(exclude_none=True)
        assert dumped == {"name": "new-name"}
        assert "data_connection_type_id" not in dumped


class TestConnectionType:
    def test_from_json(self) -> None:
        ct = ConnectionType.model_validate(SAMPLE_CONNECTION_TYPE_JSON)
        assert ct.id == "ct-1"
        assert ct.name == "postgres"
        assert ct.provider == "postgres"
        assert ct.description == "PostgreSQL connection"
        assert ct.credentials_fields == []

    def test_from_wrapped_json(self) -> None:
        """Server returns {metadata, resource} envelope."""
        ct = ConnectionType.model_validate(SAMPLE_CONNECTION_TYPE_WRAPPED_JSON)
        assert ct.id == "ct-1"
        assert ct.name == "postgres"
        assert ct.provider == "postgres"
        assert ct.description == "PostgreSQL connection"
        assert ct.tenant_id == "default"
        assert ct.created_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert ct.updated_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert ct.credentials_fields == []

    def test_credentials_field_with_enum_values(self) -> None:
        ct = ConnectionType.model_validate(
            {
                "id": "ct-s3",
                "name": "S3",
                "provider": "s3",
                "credentials_fields": [
                    {
                        "name": "region",
                        "label": "Region",
                        "required": True,
                        "type": "enum",
                        "enum_values": [
                            {"value": "us-east-1", "label": "US East"},
                            {"value": "eu-west-1", "label": "EU West"},
                        ],
                    }
                ],
            }
        )
        field = ct.credentials_fields[0]
        assert field.type == "enum"
        assert field.enum_values is not None
        assert len(field.enum_values) == 2
        assert field.enum_values[0] == EnumValue(value="us-east-1", label="US East")
        assert field.enum_values[1].value == "eu-west-1"
