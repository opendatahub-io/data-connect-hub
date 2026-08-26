use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TestCredentials {
    pub data_connection_type_id: String,
    pub secret: HashMap<String, String>,
}
