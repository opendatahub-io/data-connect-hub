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


@pytest.fixture(autouse=True)
def _no_require_fips_env(monkeypatch: pytest.MonkeyPatch) -> None:
    # A DCH_REQUIRE_FIPS set in the developer's or CI's environment would make
    # every DataConnectClient() in the suite run the FIPS checks.
    monkeypatch.delenv("DCH_REQUIRE_FIPS", raising=False)


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
        if offset >= len(data):
            raise ValueError("truncated varint")
        b = data[offset]
        offset += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            return result, offset
        shift += 7
        if shift >= 64:
            raise ValueError("varint longer than 10 bytes")


def decode_proto_fields(data: bytes) -> dict[int, bytes | int]:
    """Strict minimal protobuf decoder returning {field_number: value}.

    Supports varint and length-delimited fields, which are all the SDK sends.
    Raises ``ValueError`` for truncated data or any other wire type, so a
    malformed command fails the test instead of decoding to a plausible value.
    """
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
            if length > len(data) - i:
                raise ValueError(f"field {field_number} declares {length} bytes, {len(data) - i} remain")
            fields[field_number] = data[i : i + length]
            i += length
        else:
            raise ValueError(f"unsupported wire type {wire_type} for field {field_number}")
    return fields


def unpack_any(raw: bytes) -> tuple[str, dict[int, bytes | int]]:
    """Decode a ``google.protobuf.Any``, return (type_url, inner_fields)."""
    outer = decode_proto_fields(raw)
    type_url = outer[1]
    assert isinstance(type_url, bytes)
    inner_bytes = outer.get(2, b"")
    assert isinstance(inner_bytes, bytes)
    return type_url.decode(), decode_proto_fields(inner_bytes)
