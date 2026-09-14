use crate::api::ResourceMetadata;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone)]
pub struct FlightService {
    pub name: String,
    pub namespace: String,
    pub external_url: String,
    pub internal_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_connectors: Vec<String>,
    pub status: FlightServiceStatus,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct FlightServiceStatus {
    pub ready: bool,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct FlightServiceResource {
    pub metadata: ResourceMetadata,
    pub resource: FlightService,
}
