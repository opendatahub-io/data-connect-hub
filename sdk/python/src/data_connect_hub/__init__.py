"""Data Connect Hub Python SDK."""

from typing import TYPE_CHECKING

from ._version import __version__
from .client import DataConnectClient
from .exceptions import (
    DCHAuthenticationError,
    DCHConfigError,
    DCHConnectionError,
    DCHError,
    DCHForbiddenError,
    DCHHTTPError,
    DCHNoDataError,
    DCHNotFoundError,
    DCHQueryError,
    DCHServerError,
    DCHTimeoutError,
    DCHValidationError,
)
from .models import (
    Admin,
    AdminSecret,
    AdminSecretRef,
    ConnectionType,
    CreateConnectionRequest,
    CreateConnectionTypeRequest,
    CredentialField,
    DataConnection,
    DataConnectionState,
    DataConnectionStatus,
    DataFormat,
    EnumValue,
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)

if TYPE_CHECKING:
    from ._flight import RecordBatchStream


def __getattr__(name: str) -> object:
    if name == "RecordBatchStream":
        from ._flight import RecordBatchStream

        return RecordBatchStream
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


__all__ = [
    "Admin",
    "AdminSecret",
    "AdminSecretRef",
    "ConnectionType",
    "CreateConnectionRequest",
    "CreateConnectionTypeRequest",
    "CredentialField",
    "DCHAuthenticationError",
    "DCHConfigError",
    "DCHConnectionError",
    "DCHError",
    "DCHForbiddenError",
    "DCHHTTPError",
    "DCHNoDataError",
    "DCHNotFoundError",
    "DCHQueryError",
    "DCHServerError",
    "DCHTimeoutError",
    "DCHValidationError",
    "DataConnectClient",
    "DataConnection",
    "DataConnectionState",
    "DataConnectionStatus",
    "DataFormat",
    "EnumValue",
    "RecordBatchStream",
    "UpdateConnectionRequest",
    "UpdateConnectionTypeRequest",
    "__version__",
]
