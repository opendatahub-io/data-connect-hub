use actix_web::web::{JsonConfig, PathConfig, QueryConfig};
use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use commons::api::errors::SecretStoreError;
use commons::api::errors::{ConnectorError, MetaStoreError};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RestErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip)]
    pub status: u16,
}

#[derive(Error, Debug)]
pub enum EndpointError {
    #[error("Path not found")]
    PathNotFound,
    #[error("Header not found: {0}")]
    HeaderNotFound(String),
    #[error("Invalid header value: {0}")]
    InvalidHeaderValue(String),
    #[error("Unimplemented")]
    Unimplemented,
}

#[allow(unused)]
#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("Invalid tenant ID")]
    InvalidTenantId,
    #[error("Invalid data connection type")]
    InvalidDataConnectionType,
    #[error("Invalid secret")]
    InvalidSecret,
    #[error("Missing required key: {0}")]
    MissingRequiredKey(String),
    #[error("Flight service error: {0}")]
    FlightServiceError(String),
}

impl fmt::Display for RestErrorResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl ResponseError for RestErrorResponse {
    fn status_code(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(self)
    }
}

impl From<ConnectorError> for RestErrorResponse {
    fn from(err: ConnectorError) -> Self {
        let (code, status) = match &err {
            ConnectorError::InvalidRequest(_) => ("invalid_request", 400),
            ConnectorError::NoDataError => ("no_data", 404),
            ConnectorError::ConfigError(_) => ("config", 500),
            ConnectorError::ConnectionError(_) => ("connection", 503),
            ConnectorError::SQLError(_) => ("sql_error", 400),
            ConnectorError::IOError(_) => ("io_error", 500),
        };
        let message = match &err {
            ConnectorError::IOError(_) => {
                tracing::error!("{err}");
                "data source I/O error".to_string()
            },
            _ => err.to_string(),
        };
        RestErrorResponse {
            code: code.to_string(),
            message,
            status,
        }
    }
}

impl From<MetaStoreError> for RestErrorResponse {
    fn from(err: MetaStoreError) -> Self {
        let (code, status) = match &err {
            MetaStoreError::ResourceNotFound(_) => ("not_found", 404),
            MetaStoreError::InvalidRequest(_) => ("invalid_request", 400),
            MetaStoreError::Config(_) => ("config", 500),
            MetaStoreError::Connection(_) => ("connection", 503),
            MetaStoreError::Query(_) => ("query_error", 500),
            MetaStoreError::Conflict(_) => ("conflict", 409),
            MetaStoreError::Serialization(_) => ("serialization", 400),
            MetaStoreError::Deserialization(_) => ("deserialization", 400),
            MetaStoreError::Validation(_) => ("validation", 400),
        };
        RestErrorResponse {
            code: code.to_string(),
            message: err.to_string(),
            status,
        }
    }
}

impl From<SecretStoreError> for RestErrorResponse {
    fn from(err: SecretStoreError) -> Self {
        let (code, status) = match &err {
            SecretStoreError::SecretNotFound(_) => ("secret_not_found", 404),
            SecretStoreError::Forbidden(_) => ("forbidden", 403),
            SecretStoreError::CannotCreateSecret(_) => ("cannot_create_secret", 400),
            SecretStoreError::CannotDeleteSecret(_) => ("cannot_delete_secret", 400),
            SecretStoreError::CannotSetSecretLabels(_) => ("cannot_set_secret_labels", 400),
        };
        RestErrorResponse {
            code: code.to_string(),
            message: err.to_string(),
            status,
        }
    }
}

fn extraction_error(code: &str, err: actix_web::Error) -> actix_web::Error {
    RestErrorResponse {
        code: code.to_string(),
        message: err.to_string(),
        status: 400,
    }
    .into()
}

pub fn json_config() -> JsonConfig {
    JsonConfig::default().error_handler(|err, _req| extraction_error("invalid_json", err.into()))
}

pub fn query_config() -> QueryConfig {
    QueryConfig::default().error_handler(|err, _req| extraction_error("invalid_query", err.into()))
}

pub fn path_config() -> PathConfig {
    PathConfig::default().error_handler(|err, _req| extraction_error("invalid_path", err.into()))
}

impl From<EndpointError> for RestErrorResponse {
    fn from(err: EndpointError) -> Self {
        match err {
            EndpointError::PathNotFound => RestErrorResponse {
                code: "path_not_found".to_string(),
                message: "Path not found".to_string(),
                status: 404,
            },
            EndpointError::HeaderNotFound(header) => RestErrorResponse {
                code: "header_not_found".to_string(),
                message: format!("Header '{}' not found", header),
                status: 400,
            },
            EndpointError::InvalidHeaderValue(header) => RestErrorResponse {
                code: "invalid_header_value".to_string(),
                message: format!("Header '{}' has an invalid value", header),
                status: 400,
            },
            EndpointError::Unimplemented => RestErrorResponse {
                code: "unimplemented".to_string(),
                message: "Unimplemented".to_string(),
                status: 501,
            },
        }
    }
}

impl From<ValidationError> for RestErrorResponse {
    fn from(err: ValidationError) -> Self {
        match err {
            ValidationError::InvalidTenantId => RestErrorResponse {
                code: "invalid_tenant_id".to_string(),
                message: "Invalid tenant ID".to_string(),
                status: 400,
            },
            ValidationError::InvalidDataConnectionType => RestErrorResponse {
                code: "invalid_data_connection_type".to_string(),
                message: "Invalid data connection type".to_string(),
                status: 400,
            },
            ValidationError::InvalidSecret => RestErrorResponse {
                code: "invalid_secret".to_string(),
                message: "Invalid secret".to_string(),
                status: 400,
            },
            ValidationError::MissingRequiredKey(key) => RestErrorResponse {
                code: "missing_required_key".to_string(),
                message: format!("Missing required key: {}", key),
                status: 400,
            },
            ValidationError::FlightServiceError(error) => RestErrorResponse {
                code: "flight_service_error".to_string(),
                message: error,
                status: 500,
            },
        }
    }
}
