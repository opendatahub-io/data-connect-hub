"""Tests for Pydantic data models."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from data_connect_hub.models import (
    ConnectionType,
    CreateConnectionRequest,
    CredentialsRef,
    DataConnection,
    DataConnectionStatus,
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
        assert conn.credentials_ref == CredentialsRef(secret="secret/test-conn")
        assert conn.created_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.updated_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.properties == {"key": "value"}

    def test_from_wrapped_json(self) -> None:
        """Server returns {metadata, resource, status} envelope."""
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_WRAPPED_JSON)
        assert conn.id == "123"
        assert conn.name == "test-conn"
        assert conn.data_connection_type_id == "postgres"
        assert conn.format == "tabular"
        assert conn.tenant_id == "tenant-1"
        assert conn.credentials_ref == CredentialsRef(secret="secret/test-conn")
        assert conn.created_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.updated_at == datetime(2026, 1, 1, tzinfo=UTC)
        assert conn.properties == {"key": "value"}
        assert conn.status.state == "ready"
        assert conn.status.message == "Connected"

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

    def test_default_status(self) -> None:
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        assert conn.status.state == "not_ready"
        assert conn.status.message is None

    def test_status_from_flat_json(self) -> None:
        data = {**SAMPLE_CONNECTION_JSON, "status": {"state": "ready", "message": "OK"}}
        conn = DataConnection.model_validate(data)
        assert conn.status.state == "ready"
        assert conn.status.message == "OK"

    def test_default_tenant_id(self) -> None:
        data = dict(SAMPLE_CONNECTION_JSON)
        del data["tenant_id"]
        conn = DataConnection.model_validate(data)
        assert conn.tenant_id == ""

    def test_wrapped_json_without_status(self) -> None:
        data = dict(SAMPLE_CONNECTION_WRAPPED_JSON)
        del data["status"]
        conn = DataConnection.model_validate(data)
        assert conn.status == DataConnectionStatus()


class TestDataConnectionStatus:
    @pytest.mark.parametrize("state", ["ready", "ingestion_not_ready", "not_ready"])
    def test_accepts_every_server_state(self, state: str) -> None:
        """All three ``commons::api::connections::DataConnectionState`` variants."""
        data = {**SAMPLE_CONNECTION_JSON, "status": {"state": state}}
        conn = DataConnection.model_validate(data)
        assert conn.status.state == state

    def test_status_updated_at_is_parsed(self) -> None:
        data = {
            **SAMPLE_CONNECTION_JSON,
            "status": {"state": "ready", "message": "OK", "updated_at": "2026-02-03T04:05:06Z"},
        }
        conn = DataConnection.model_validate(data)
        assert conn.status.updated_at == datetime(2026, 2, 3, 4, 5, 6, tzinfo=UTC)

    def test_status_updated_at_defaults_to_none(self) -> None:
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        assert conn.status.updated_at is None


class TestAdmin:
    def test_credentials_ref_round_trip(self) -> None:
        creds = CredentialsRef(secret="secret/my-conn")
        dumped = creds.model_dump()
        restored = CredentialsRef.model_validate(dumped)
        assert restored == creds


class TestDataConnectionRepr:
    def test_repr_masks_properties(self) -> None:
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        text = repr(conn)
        assert "value" not in text
        assert "***" in text
        assert "key" in text

    def test_repr_credentials_ref_present(self) -> None:
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        text = repr(conn)
        assert "credentials_ref" in text

    def test_repr_empty_properties_not_masked(self) -> None:
        data = dict(SAMPLE_CONNECTION_JSON)
        data["properties"] = {}
        conn = DataConnection.model_validate(data)
        text = repr(conn)
        assert "***" not in text


class TestCreateConnectionRequest:
    def test_dump_includes_credentials_ref(self) -> None:
        req = CreateConnectionRequest(
            name="conn",
            data_connection_type_id="postgres",
            format="tabular",
            credentials_ref=CredentialsRef(secret="secret/test"),
        )
        dumped = req.model_dump(exclude_none=True)
        assert dumped["data_connection_type_id"] == "postgres"
        assert dumped["properties"] == {}
        assert dumped["credentials_ref"] == {"secret": "secret/test"}

    def test_repr_masks_properties(self) -> None:
        req = CreateConnectionRequest(
            name="conn",
            data_connection_type_id="postgres",
            format="tabular",
            credentials_ref=CredentialsRef(secret="secret/test"),
            properties={"host": "db.internal", "password": "secret123"},
        )
        text = repr(req)
        assert "db.internal" not in text
        assert "secret123" not in text
        assert "***" in text


class TestUpdateConnectionRequest:
    def test_partial_dump(self) -> None:
        req = UpdateConnectionRequest(name="new-name")
        dumped = req.model_dump(exclude_none=True)
        assert dumped == {"name": "new-name"}
        assert "data_connection_type_id" not in dumped

    def test_repr_masks_properties(self) -> None:
        req = UpdateConnectionRequest(properties={"host": "db.internal"})
        text = repr(req)
        assert "db.internal" not in text
        assert "***" in text


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

    def test_capabilities_from_wrapped_status(self) -> None:
        data = {**SAMPLE_CONNECTION_TYPE_WRAPPED_JSON, "status": {"capabilities": {"flight": True, "rest": False}}}
        ct = ConnectionType.model_validate(data)
        assert ct.status.capabilities.flight is True
        assert ct.status.capabilities.rest is False

    def test_capabilities_default_to_false(self) -> None:
        ct = ConnectionType.model_validate(SAMPLE_CONNECTION_TYPE_WRAPPED_JSON)
        assert ct.status.capabilities.flight is False
        assert ct.status.capabilities.rest is False

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
