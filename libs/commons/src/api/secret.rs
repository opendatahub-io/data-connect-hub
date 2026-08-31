use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct Secret {
    pub name: String,
    pub namespace: String,
    pub properties: HashMap<String, String>,
    pub labels: Option<HashMap<String, String>>,
    pub annotations: Option<HashMap<String, String>>,
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret")
            .field("name", &self.name)
            .field("namespace", &self.namespace)
            .field("properties", &"[REDACTED]")
            .field("labels", &self.labels)
            .finish()
    }
}
