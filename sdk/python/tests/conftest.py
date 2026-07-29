"""Shared test fixtures."""

from __future__ import annotations

import pytest

from data_connect_hub.models import DataConnection, DataLocation

SAMPLE_CONNECTION_JSON = {
    "id": "123",
    "namespace": "test-ns",
    "name": "test-conn",
    "provider": "postgres",
    "format": "jdbc",
    "tenant_id": "tenant-1",
    "location": {"url": "postgresql://localhost:5432/db"},
    "created_at": "2026-01-01T00:00:00Z",
    "updated_at": "2026-01-01T00:00:00Z",
    "properties": {"key": "value"},
}

SAMPLE_CONNECTION_TYPE_JSON = {
    "id": "ct-1",
    "name": "postgres",
    "description": "PostgreSQL connection",
    "properties_schema": {"host": "string", "port": "integer"},
}


@pytest.fixture()
def sample_connection() -> DataConnection:
    return DataConnection.model_validate(SAMPLE_CONNECTION_JSON)


@pytest.fixture()
def sample_connection_json() -> dict[str, object]:
    return dict(SAMPLE_CONNECTION_JSON)


@pytest.fixture()
def sample_location() -> DataLocation:
    return DataLocation(url="postgresql://localhost:5432/db")
