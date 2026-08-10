pub mod auth;
pub mod errors;
pub mod registry;
pub mod service;

use commons::api::connections::{Secret, SecretStore};
use commons::api::errors::SecretStoreError;
pub use service::*;
use std::collections::HashMap;

pub struct InMemorySecretStore {
    secrets: HashMap<String, Secret>,
}

impl InMemorySecretStore {
    pub fn new(secrets: Vec<Secret>) -> Self {
        Self {
            secrets: secrets
                .into_iter()
                .map(|secret| (format!("{}/{}", secret.namespace, secret.name), secret))
                .collect(),
        }
    }
}

#[async_trait::async_trait]
impl SecretStore for InMemorySecretStore {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError> {
        self.secrets
            .get(format!("{namespace}/{name}").as_str())
            .cloned()
            .ok_or(SecretStoreError::SecretNotFound(format!(
                "Secret not found: {namespace}/{name}"
            )))
    }
}
