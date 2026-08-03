pub mod endpoints;
pub mod errors;
pub mod middleware;
use serde_json::Value;
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonPatch {
    pub op: String,
    pub path: String,
    pub value: Option<Value>,
}

