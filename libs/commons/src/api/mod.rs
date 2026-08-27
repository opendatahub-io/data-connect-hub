pub mod connection_types;
pub mod connections;
pub mod connector;
pub mod creds;
pub mod errors;
pub mod storage;

pub const X_DATA_CONNECTION_ID: &str = "x-data-connection-id";
pub const X_TENANT_ID: &str = "x-tenant-id";
pub const X_REMOTE_USER: &str = "x-remote-user";
pub const X_REMOTE_GROUPS: &str = "x-remote-groups";

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceMetadata {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceList<T> {
    pub total_count: usize,
    pub items: Vec<T>,
}
