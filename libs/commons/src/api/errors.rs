use thiserror::Error;

/// Only connector implementations should emit this error.
#[derive(Error, Debug, Clone)]
pub enum ConnectorError {
    #[error("SQL error: {0}")]
    SQLError(String),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("No data returned")]
    NoDataError,
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Config error: {0}")]
    ConfigError(String),
    #[error("IO error: {0}")]
    IOError(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(String),
}

impl ConnectorError {
    pub fn message(&self) -> &str {
        match self {
            Self::SQLError(m)
            | Self::ConnectionError(m)
            | Self::InvalidRequest(m)
            | Self::ConfigError(m)
            | Self::IOError(m)
            | Self::NotFound(m)
            | Self::UnsupportedOperation(m) => m,
            Self::NoDataError => "no data returned",
        }
    }
}

/// Only meta store implementations should emit this error.
#[derive(Error, Debug)]
pub enum MetaStoreError {
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Not found: {0}")]
    ResourceNotFound(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Config error: {0}")]
    Config(String),
    #[error("Query error: {0}")]
    Query(String),
    #[error("Resource conflict: {0}")]
    Conflict(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Deserialization error: {0}")]
    Deserialization(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Unprocessable entity: {0}")]
    UnprocessableEntity(String),
}

/// Only secret store implementations should emit this error.
#[derive(Error, Debug, Clone)]
pub enum SecretStoreError {
    #[error("Secret not found: {0}")]
    SecretNotFound(String),
    #[error("Access denied: {0}")]
    Forbidden(String),
    #[error("Cannot create secret '{name}' in namespace '{namespace}': {reason}")]
    CannotCreateSecret {
        name: String,
        namespace: String,
        reason: String,
        kind: CreateSecretErrorKind,
    },
    #[error("Cannot delete secret: {0}")]
    CannotDeleteSecret(String),
    #[error("Cannot set secret labels: {0}")]
    CannotSetSecretLabels(String),
}

/// Why a secret write failed, from the caller's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateSecretErrorKind {
    /// The secret already exists, or its fields are managed by another client.
    /// The caller can retry with a different secret name.
    Conflict,
    /// The secret name or keys are invalid, or the tenant namespace does not
    /// exist. The caller can fix the request.
    Invalid,
    /// The Kubernetes API is unreachable or failed. The caller can retry later.
    Unavailable,
    /// The write was rejected for a reason the caller cannot fix, such as
    /// missing RBAC for the DCH service account or an admission policy.
    Internal,
}

/// Errors pertainint to data connection types.
#[derive(Error, Debug)]
pub enum DataConnectionTypeError {
    #[error("Required field {0} is missing")]
    MissingRequiredField(String),
    #[error("{field} must not be empty or whitespace-only{}", index.map(|i| format!(" (index {i})")).unwrap_or_default())]
    EmptyValue { field: &'static str, index: Option<usize> },
}
