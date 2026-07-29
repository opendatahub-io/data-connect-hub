"""Pydantic v2 models mirroring commons::api::connections Rust types.

Note on ``tenant_id``: The server injects ``tenant_id`` into persisted
records from the ``x-tenant-id`` HTTP header.  Create/Update request models
therefore omit this field — it is never sent by the client.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from pydantic import BaseModel, ConfigDict, Field


class DataLocation(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    url: str


class DataConnection(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    id: str
    namespace: str
    name: str
    provider: str
    format: str  # mirrors the API field name; use `data_format` kwarg on the client
    tenant_id: str
    location: DataLocation
    created_at: datetime
    updated_at: datetime
    properties: dict[str, str] = Field(default_factory=dict)


class CreateConnectionRequest(BaseModel):
    namespace: str
    name: str
    provider: str
    format: str
    location: DataLocation
    properties: dict[str, str] = Field(default_factory=dict)


class UpdateConnectionRequest(BaseModel):
    name: str | None = None
    namespace: str | None = None
    provider: str | None = None
    format: str | None = None
    location: DataLocation | None = None
    properties: dict[str, str] | None = None


class ConnectionType(BaseModel):
    id: str
    name: str
    description: str = ""
    properties_schema: dict[str, Any] = Field(default_factory=dict)


class CreateConnectionTypeRequest(BaseModel):
    name: str
    description: str = ""
    properties_schema: dict[str, Any] = Field(default_factory=dict)


class UpdateConnectionTypeRequest(BaseModel):
    name: str | None = None
    description: str | None = None
    properties_schema: dict[str, Any] | None = None
