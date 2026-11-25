"""Shared test fixtures."""

from __future__ import annotations

import pytest

from data_connect_hub.models import DataConnection

SAMPLE_CONNECTION_JSON = {
    "id": "123",
    "name": "test-conn",
    "data_connection_type_id": "postgres",
    "format": "tabular",
    "tenant_id": "tenant-1",
    "credentials_ref": {"secret": "secret/test-conn"},
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
        "name": "test-conn",
        "data_connection_type_id": "postgres",
        "format": "tabular",
        "credentials_ref": {"secret": "secret/test-conn"},
        "properties": {"key": "value"},
    },
    "status": {
        "state": "ready",
        "message": "Connected",
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


# -- protobuf wire-format decoding (independent of the SDK encoder) -----------


def read_varint(data: bytes, offset: int) -> tuple[int, int]:
    """Read a protobuf varint from *data* at *offset*, return (value, new_offset)."""
    result = 0
    shift = 0
    while True:
        b = data[offset]
        offset += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            break
        shift += 7
    return result, offset


def decode_proto_fields(data: bytes) -> dict[int, bytes | int]:
    """Minimal protobuf wire-format decoder returning {field_number: value}."""
    fields: dict[int, bytes | int] = {}
    i = 0
    while i < len(data):
        tag, i = read_varint(data, i)
        field_number = tag >> 3
        wire_type = tag & 0x7
        if wire_type == 0:
            value, i = read_varint(data, i)
            fields[field_number] = value
        elif wire_type == 2:
            length, i = read_varint(data, i)
            fields[field_number] = data[i : i + length]
            i += length
    return fields


def unpack_any(raw: bytes) -> tuple[str, dict[int, bytes | int]]:
    """Decode a ``google.protobuf.Any``, return (type_url, inner_fields)."""
    outer = decode_proto_fields(raw)
    type_url = outer[1]
    assert isinstance(type_url, bytes)
    inner_bytes = outer.get(2, b"")
    assert isinstance(inner_bytes, bytes)
    return type_url.decode(), decode_proto_fields(inner_bytes)
