use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApiError {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_display() {
        assert_eq!(
            ApiError::SQLError("table not found".into()).to_string(),
            "SQL error: table not found"
        );
        assert_eq!(
            ApiError::ConnectionError("timeout".into()).to_string(),
            "Connection error: timeout"
        );
        assert_eq!(ApiError::NoDataError().to_string(), "No data returned");
        assert_eq!(
            ApiError::InvalidRequest("missing field".into()).to_string(),
            "Invalid request: missing field"
        );
        assert_eq!(
            ApiError::ConfigError("bad value".into()).to_string(),
            "Config error: bad value"
        );
    }
}
