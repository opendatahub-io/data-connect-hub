"""Shared test fixtures."""

from __future__ import annotations

import pytest

from data_connect_hub.models import DataConnection, DataLocation

SAMPLE_CONNECTION_JSON = {
    "id": "123",
    "namespace": "test-ns",
    "name": "test-conn",
    "provider": "postgres",
    "format": "tabular",
    "tenant_id": "tenant-1",
    "location": {"url": "postgresql://localhost:5432/db"},
    "created_at": "2026-01-01T00:00:00Z",
    "updated_at": "2026-01-01T00:00:00Z",
    "properties": {"key": "value"},
}

SAMPLE_CONNECTION_WRAPPED_JSON = {
    "metadata": {
        "id": "123",
        "tenant_id": "tenant-1",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    },
    "resource": {
        "namespace": "test-ns",
        "name": "test-conn",
        "provider": "postgres",
        "format": "tabular",
        "location": {"url": "postgresql://localhost:5432/db"},
        "properties": {"key": "value"},
    },
}

SAMPLE_CONNECTION_TYPE_JSON = {
    "id": "ct-1",
    "name": "postgres",
    "provider": "postgres",
    "description": "PostgreSQL connection",
    "credentials_fields": [],
}

SAMPLE_CONNECTION_TYPE_WRAPPED_JSON = {
    "metadata": {
        "id": "ct-1",
        "tenant_id": "default",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    },
    "resource": {
        "name": "postgres",
        "provider": "postgres",
        "description": "PostgreSQL connection",
        "credentials_fields": [],
    },
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
