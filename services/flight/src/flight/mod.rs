mod actions;
pub mod auth;
pub mod errors;
pub mod metrics;
pub mod registry;
pub mod service;

pub use service::*;

use commons::api::X_DATA_CONNECTION_ID;
use commons::api::X_TENANT_ID;
use tonic::Status;
use tonic::metadata::MetadataMap;

#[derive(Debug, Clone)]
pub(crate) struct QueryContext {
    pub tenant_id: String,
    pub connection_id: String,
}

impl QueryContext {
    pub fn from_request(metadata: &MetadataMap) -> Result<Self, Status> {
        let tenant_id = metadata
            .get(X_TENANT_ID)
            .ok_or(Status::invalid_argument(format!("{X_TENANT_ID} header is required")))?
            .to_str()
            .map_err(|_| Status::invalid_argument(format!("{X_TENANT_ID} header must be valid ASCII")))?;
        let connection_id = metadata
            .get(X_DATA_CONNECTION_ID)
            .ok_or(Status::invalid_argument(format!(
                "{X_DATA_CONNECTION_ID} header is required"
            )))?
            .to_str()
            .map_err(|_| Status::invalid_argument(format!("{X_DATA_CONNECTION_ID} header must be valid ASCII")))?;
        Ok(Self {
            tenant_id: tenant_id.to_string(),
            connection_id: connection_id.to_string(),
        })
    }
}
