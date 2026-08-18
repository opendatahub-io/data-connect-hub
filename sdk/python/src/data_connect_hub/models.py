"""Pydantic v2 models mirroring commons::api::connections Rust types.

The server returns resources wrapped in ``{metadata, resource}`` envelopes.
The SDK flattens these into user-friendly models that merge metadata fields
(id, tenant_id, created_at, updated_at) with the resource fields.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

DataFormat = Literal["tabular", "binary"]
DataConnectionState = Literal["ready", "not_ready"]


class AdminSecretRef(BaseModel):
    secret_ref: str


class AdminSecret(BaseModel):
    name: str
    secret: dict[str, str]


Admin = AdminSecretRef | AdminSecret


class DataConnectionStatus(BaseModel):
    state: DataConnectionState = "not_ready"
    message: str | None = None
    phases: list[dict[str, Any]] = Field(default_factory=list)


class DataConnection(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    id: str
    name: str
    data_connection_type_id: str
    format: DataFormat
    tenant_id: str = ""
    created_at: datetime
    updated_at: datetime
    admin: Admin | None = None
    properties: dict[str, str] = Field(default_factory=dict)
    status: DataConnectionStatus = Field(default_factory=DataConnectionStatus)

    @model_validator(mode="before")
    @classmethod
    def _flatten_resource(cls, data: Any) -> Any:
        if isinstance(data, dict) and "metadata" in data and "resource" in data:
            flat = {**data["metadata"], **data["resource"]}
            if "status" in data:
                flat["status"] = data["status"]
            return flat
        return data


class CreateConnectionRequest(BaseModel):
    name: str
    data_connection_type_id: str
    format: DataFormat
    admin: Admin | None = None
    properties: dict[str, str] = Field(default_factory=dict)


class UpdateConnectionRequest(BaseModel):
    name: str | None = None
    data_connection_type_id: str | None = None
    format: DataFormat | None = None
    admin: Admin | None = None
    properties: dict[str, str] | None = None


class EnumValue(BaseModel):
    value: str
    label: str


class CredentialField(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    name: str
    label: str
    description: str | None = None
    required: bool
    type: str
    enum_values: list[EnumValue] | None = None
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
