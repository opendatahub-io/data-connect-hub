use thiserror::Error;

/// Only connector implementations should emit this error.
#[derive(Error, Debug)]
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
    #[error("Cannot create secret: {0}")]
    CannotCreateSecret(String),
    #[error("Cannot delete secret: {0}")]
    CannotDeleteSecret(String),
    #[error("Cannot set secret labels: {0}")]
    CannotSetSecretLabels(String),
}

/// Errors pertainint to data connection types.
#[derive(Error, Debug)]
pub enum DataConnectionTypeError {
    #[error("Required field {0} is missing")]
    MissingRequiredField(String),
}
