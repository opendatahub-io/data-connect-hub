"""Pydantic v2 models mirroring commons::api::connections Rust types.

The server returns resources wrapped in ``{metadata, resource}`` envelopes.
The SDK flattens these into user-friendly models that merge metadata fields
(id, tenant_id, created_at, updated_at) with the resource fields.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from pydantic import BaseModel, ConfigDict, Field, model_validator


class DataLocation(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    url: str


class DataConnection(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    id: str
    namespace: str
    name: str
    provider: str
    format: str
    tenant_id: str
    location: DataLocation
    created_at: datetime
    updated_at: datetime
    properties: dict[str, str] = Field(default_factory=dict)

    @model_validator(mode="before")
    @classmethod
    def _flatten_resource(cls, data: Any) -> Any:
        if isinstance(data, dict) and "metadata" in data and "resource" in data:
            flat = {**data["metadata"], **data["resource"]}
            return flat
        return data


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


class CredentialField(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    name: str
    label: str
    description: str | None = None
    required: bool = False
    type: str = "string"
    enum_values: list[dict[str, str]] | None = None
    default_value: str | None = None


class ConnectionType(BaseModel):
    id: str
    name: str
    provider: str
    description: str | None = None
    tenant_id: str = ""
    created_at: datetime | None = None
    updated_at: datetime | None = None
    credentials_fields: list[CredentialField] = Field(default_factory=list)

    @model_validator(mode="before")
    @classmethod
    def _flatten_resource(cls, data: Any) -> Any:
        if isinstance(data, dict) and "metadata" in data and "resource" in data:
            flat = {**data["metadata"], **data["resource"]}
            return flat
        return data


class CreateConnectionTypeRequest(BaseModel):
    name: str
    provider: str
    description: str | None = None
    credentials_fields: list[CredentialField] = Field(default_factory=list)


class UpdateConnectionTypeRequest(BaseModel):
    name: str | None = None
    provider: str | None = None
    description: str | None = None
    credentials_fields: list[CredentialField] | None = None
