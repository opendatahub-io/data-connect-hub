pub mod endpoints;
pub mod errors;
pub mod middleware;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonPatch {
    pub op: String,
    pub path: String,
    pub value: Option<Value>,
}
