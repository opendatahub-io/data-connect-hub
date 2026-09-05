"""Data Connect Hub Python SDK."""

from importlib.metadata import PackageNotFoundError, version

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
    DCHResponseError,
    DCHServerError,
    DCHTimeoutError,
    DCHValidationError,
)
from .models import (
    Capabilities,
    ConnectionType,
    ConnectionTypeStatus,
    CreateConnectionRequest,
    CreateConnectionTypeRequest,
    CredentialField,
    CredentialsRef,
    DataConnection,
    DataConnectionState,
    DataConnectionStatus,
    DataFormat,
    EnumValue,
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)

try:
    __version__ = version("data-connect-hub")
except PackageNotFoundError:
    # Imported from a source tree without an installed distribution. Keep this
    # PEP 440 parseable so callers can compare it without special-casing.
    __version__ = "0.0.0+unknown"

__all__ = [
    "Capabilities",
    "ConnectionType",
    "ConnectionTypeStatus",
    "CreateConnectionRequest",
    "CreateConnectionTypeRequest",
    "CredentialField",
    "CredentialsRef",
    "DCHAuthenticationError",
    "DCHConfigError",
    "DCHConnectionError",
    "DCHError",
    "DCHForbiddenError",
    "DCHHTTPError",
    "DCHNoDataError",
    "DCHNotFoundError",
    "DCHQueryError",
    "DCHResponseError",
    "DCHServerError",
    "DCHTimeoutError",
    "DCHValidationError",
    "DataConnectClient",
    "DataConnection",
    "DataConnectionState",
    "DataConnectionStatus",
    "DataFormat",
    "EnumValue",
    "UpdateConnectionRequest",
    "UpdateConnectionTypeRequest",
    "__version__",
]
