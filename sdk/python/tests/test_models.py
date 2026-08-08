"""Tests for Pydantic data models."""

from __future__ import annotations

from datetime import UTC, datetime

from data_connect_hub.models import (
    ConnectionType,
    CreateConnectionRequest,
    DataConnection,
    DataLocation,
    UpdateConnectionRequest,
)

from .conftest import SAMPLE_CONNECTION_JSON, SAMPLE_CONNECTION_TYPE_JSON


class TestDataLocation:
    def test_create(self) -> None:
        loc = DataLocation(url="postgresql://localhost:5432/db")
        assert loc.url == "postgresql://localhost:5432/db"

    def test_round_trip(self) -> None:
        loc = DataLocation(url="s3://bucket/path")
        dumped = loc.model_dump()
        restored = DataLocation.model_validate(dumped)
        assert restored.url == loc.url


class TestDataConnection:
    def test_from_json_fixture(self) -> None:
        """Mirrors the Rust test in commons/src/api/connections.rs."""
        conn = DataConnection.model_validate(SAMPLE_CONNECTION_JSON)
        assert conn.id == "123"
        assert conn.namespace == "test-ns"
        assert conn.name == "test-conn"
        assert conn.provider == "postgres"
        assert conn.format == "tabular"
        assert conn.tenant_id == "tenant-1"
        assert conn.location.url == "postgresql://localhost:5432/db"
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


class TestCreateConnectionRequest:
    def test_dump_excludes_none(self) -> None:
        req = CreateConnectionRequest(
            namespace="ns",
            name="conn",
            provider="postgres",
            format="arrow",
            location=DataLocation(url="pg://localhost"),
        )
        dumped = req.model_dump(exclude_none=True)
        assert "namespace" in dumped
        assert dumped["properties"] == {}


class TestUpdateConnectionRequest:
    def test_partial_dump(self) -> None:
        req = UpdateConnectionRequest(name="new-name")
        dumped = req.model_dump(exclude_none=True)
        assert dumped == {"name": "new-name"}
        assert "namespace" not in dumped
        assert "provider" not in dumped


class TestConnectionType:
    def test_from_json(self) -> None:
        ct = ConnectionType.model_validate(SAMPLE_CONNECTION_TYPE_JSON)
        assert ct.id == "ct-1"
        assert ct.name == "postgres"
        assert ct.description == "PostgreSQL connection"
        assert ct.properties_schema == {"host": "string", "port": "integer"}
