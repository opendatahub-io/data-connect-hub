use thiserror::Error;

#[derive(Error, Debug)]
pub enum MetaStoreError {
    #[error("Connection error: {0}")]
    Connection(String),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metastore_error_display() {
        assert_eq!(
            MetaStoreError::Connection("refused".into()).to_string(),
            "Connection error: refused"
        );
        assert_eq!(
            MetaStoreError::InvalidRequest("bad id".into()).to_string(),
            "Invalid request: bad id"
        );
        assert_eq!(
            MetaStoreError::Config("missing key".into()).to_string(),
            "Config error: missing key"
        );
        assert_eq!(
            MetaStoreError::Query("syntax error".into()).to_string(),
            "Query error: syntax error"
        );
        assert_eq!(
            MetaStoreError::Serialization("failed".into()).to_string(),
            "Serialization error: failed"
        );
        assert_eq!(
            MetaStoreError::Deserialization("invalid json".into()).to_string(),
            "Deserialization error: invalid json"
        );
    }
}
