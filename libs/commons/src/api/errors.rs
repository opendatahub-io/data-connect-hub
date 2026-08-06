use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConnectorError {
    #[error("SQL error: {0}")]
    SQLError(String),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("No data returned")]
    NoDataError(),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Config error: {0}")]
    ConfigError(String),
}

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
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Deserialization error: {0}")]
    Deserialization(String),
    #[error("Validation error: {0}")]
    Validation(String),
}

#[derive(Error, Debug)]
pub enum SecretStoreError {
    #[error("Secret not found: {0}")]
    SecretNotFound(String),
    #[error("Access denied: {0}")]
    Forbidden(String),
}
