use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use commons::api::errors::{ConnectorError, MetaStoreError};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RestError {
    pub code: String,
    pub message: String,
    #[serde(skip)]
    pub status: u16,
}

impl fmt::Display for RestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl ResponseError for RestError {
    fn status_code(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(self)
    }
}

impl From<ConnectorError> for RestError {
    fn from(err: ConnectorError) -> Self {
        let (code, status) = match &err {
            ConnectorError::InvalidRequest(_) => ("invalid_request", 400),
            ConnectorError::NoDataError() => ("no_data", 404),
            ConnectorError::ConfigError(_) => ("config", 500),
            ConnectorError::ConnectionError(_) => ("connection", 503),
            ConnectorError::SQLError(_) => ("sql_error", 400),
        };
        RestError {
            code: code.to_string(),
            message: err.to_string(),
            status,
        }
    }
}

impl From<MetaStoreError> for RestError {
    fn from(err: MetaStoreError) -> Self {
        let (code, status) = match &err {
            MetaStoreError::InvalidRequest(_) => ("invalid_request", 400),
            MetaStoreError::Config(_) => ("config", 500),
            MetaStoreError::Connection(_) => ("connection", 503),
            MetaStoreError::Query(_) => ("query_error", 400),
            MetaStoreError::Serialization(_) => ("serialization", 500),
            MetaStoreError::Deserialization(_) => ("deserialization", 400),
        };
        RestError {
            code: code.to_string(),
            message: err.to_string(),
            status,
        }
    }
}
