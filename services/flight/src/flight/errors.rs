use commons::api::errors::{ConnectorError, MetaStoreError, SecretStoreError};
use tonic::Status;

pub(crate) fn map_meta_store_error(e: MetaStoreError) -> Status {
    match e {
        MetaStoreError::ResourceNotFound(msg) => Status::not_found(msg),
        MetaStoreError::InvalidRequest(msg) => Status::invalid_argument(msg),
        MetaStoreError::Validation(msg) => Status::invalid_argument(msg),
        e @ MetaStoreError::Connection(_) => {
            tracing::error!(error = %e, "metadata store connection failed");
            Status::unavailable("metadata service unavailable")
        },
        e @ MetaStoreError::Config(_) => {
            tracing::error!(error = %e, "metadata store config error");
            Status::internal("metadata service configuration error")
        },
        e @ MetaStoreError::Query(_) => {
            tracing::error!(error = %e, "metadata store query failed");
            Status::internal("metadata query error")
        },
        e @ MetaStoreError::Conflict(_) => {
            tracing::warn!(error = %e, "metadata store conflict");
            Status::already_exists("resource already exists")
        },
        e @ MetaStoreError::Serialization(_) => {
            tracing::error!(error = %e, "metadata serialization failed");
            Status::internal("metadata serialization error")
        },
        e @ MetaStoreError::Deserialization(_) => {
            tracing::error!(error = %e, "metadata deserialization failed");
            Status::internal("metadata deserialization error; see service logs for details")
        },
    }
}

pub(crate) fn map_secret_store_error(e: SecretStoreError) -> Status {
    match e {
        SecretStoreError::SecretNotFound(msg) => Status::not_found(msg),
        SecretStoreError::Forbidden(msg) => Status::permission_denied(msg),
        e @ SecretStoreError::CannotCreateSecret(_) => {
            tracing::error!(error = %e, "failed to create secret");
            Status::internal("cannot create secret")
        },
        e @ SecretStoreError::CannotDeleteSecret(_) => {
            tracing::error!(error = %e, "failed to delete secret");
            Status::internal("cannot delete secret")
        },
        e @ SecretStoreError::CannotSetSecretLabels(_) => {
            tracing::error!(error = %e, "failed to set secret labels");
            Status::internal("cannot set secret labels")
        },
    }
}

pub(crate) fn map_connector_error(e: ConnectorError) -> Status {
    match e {
        ConnectorError::InvalidRequest(msg) => Status::invalid_argument(msg),
        ConnectorError::NoDataError => Status::not_found("no data found"),
        e @ ConnectorError::ConnectionError(_) => {
            tracing::error!(error = %e, "connector connection failed");
            Status::unavailable("data source connection failed")
        },
        e @ ConnectorError::SQLError(_) => {
            tracing::error!(error = %e, "connector SQL error");
            Status::internal("query execution failed")
        },
        e @ ConnectorError::ConfigError(_) => {
            tracing::error!(error = %e, "connector config error");
            Status::internal("connector configuration error")
        },
        e @ ConnectorError::IOError(_) => {
            tracing::error!(error = %e, "connector IO error");
            Status::internal("data source IO error")
        },
    }
}
