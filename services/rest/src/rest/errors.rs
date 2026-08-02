use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use commons::api::errors::{ConnectorError, MetaStoreError};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RestError {
    pub code: String,
    pub message: String,
    #[serde(skip)]
    pub status: u16,
}

#[derive(Error, Debug)]
pub enum EndpointError {
    #[error("Path not found")]
    PathNotFound,
    #[error("Invalid payload")]
    InvalidPayload,
    #[error("Header not found: {0}")]
    HeaderNotFound(String),
    #[error("Invalid header value: {0}")]
    InvalidHeaderValue(String),
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
            MetaStoreError::ResourceNotFound(_) => ("not_found", 404),
            MetaStoreError::InvalidRequest(_) => ("invalid_request", 400),
            MetaStoreError::Config(_) => ("config", 500),
            MetaStoreError::Connection(_) => ("connection", 503),
            MetaStoreError::Query(_) => ("query_error", 400),
            MetaStoreError::Serialization(_) => ("serialization", 400),
            MetaStoreError::Deserialization(_) => ("deserialization", 400),
        };
        RestError {
            code: code.to_string(),
            message: err.to_string(),
            status,
        }
    }
}

impl From<EndpointError> for RestError {
    fn from(err: EndpointError) -> Self {
        match err {
            EndpointError::PathNotFound => RestError {
                code: "path_not_found".to_string(),
                message: "Path not found".to_string(),
                status: 404,
            },
            EndpointError::InvalidPayload => RestError {
                code: "invalid_payload".to_string(),
                message: "Invalid payload".to_string(),
                status: 400,
            },
            EndpointError::HeaderNotFound(header) => RestError {
                code: "header_not_found".to_string(),
                message: format!("Header '{}' not found", header),
                status: 400,
            },
            EndpointError::InvalidHeaderValue(header) => RestError {
                code: "invalid_header_value".to_string(),
                message: format!("Header '{}' has an invalid value", header),
                status: 400,
            },
        }
    }
}
