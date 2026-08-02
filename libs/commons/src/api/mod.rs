pub mod connections;
pub mod errors;
pub mod tabular;

pub static X_DATA_CONNECTION_ID: &str = "x-data-connection-id";
pub static X_TENANT_ID: &str = "x-tenant-id";

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceMetadata {
    pub id: String,
    pub tenant_id: String,
    pub created_at: String,
    pub updated_at: String,
}
