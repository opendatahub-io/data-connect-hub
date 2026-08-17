use commons::api::errors::{ConnectorError, MetaStoreError, SecretStoreError};
use tonic::Status;

pub(crate) fn map_meta_store_error(e: MetaStoreError) -> Status {
    match e {
        MetaStoreError::ResourceNotFound(_) => Status::not_found("resource not found"),
        MetaStoreError::InvalidRequest(_) => Status::invalid_argument("invalid metadata request"),
        MetaStoreError::Connection(_) => Status::unavailable("metadata service unavailable"),
        MetaStoreError::Config(_) => Status::internal("metadata service configuration error"),
        MetaStoreError::Query(_) => Status::internal("metadata query error"),
        MetaStoreError::Conflict(_) => Status::already_exists("resource already exists"),
        MetaStoreError::Serialization(_) => Status::invalid_argument("invalid metadata payload"),
        MetaStoreError::Deserialization(_) => Status::invalid_argument("invalid metadata payload"),
        MetaStoreError::Validation(_) => Status::invalid_argument("validation error"),
    }
}

pub(crate) fn map_secret_store_error(e: SecretStoreError) -> Status {
    match e {
        SecretStoreError::SecretNotFound(_) => Status::not_found("secret not found"),
        SecretStoreError::Forbidden(_) => Status::permission_denied("access denied"),
        SecretStoreError::CannotCreateSecret(_) => Status::internal("cannot create secret"),
        SecretStoreError::CannotDeleteSecret(_) => Status::internal("cannot delete secret"),
        SecretStoreError::CannotSetSecretLabels(_) => Status::internal("cannot set secret labels"),
    }
}

pub(crate) fn map_connector_error(e: ConnectorError) -> Status {
    match e {
        ConnectorError::InvalidRequest(_) => Status::invalid_argument("invalid request"),
        ConnectorError::ConnectionError(_) => Status::unavailable("data source connection failed"),
        ConnectorError::NoDataError() => Status::not_found("no data found"),
        ConnectorError::SQLError(_) => Status::invalid_argument("query rejected"),
        ConnectorError::ConfigError(_) => Status::internal("connector configuration error"),
        ConnectorError::IOError(_) => {
            tracing::error!("{e}");
            Status::internal("data source IO error")
        },
    }
}
