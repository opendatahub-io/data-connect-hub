use commons::api::connection_types::Secret;
use commons::api::errors::SecretStoreError;
use commons::api::storage::SecretStore;
use std::collections::HashMap;
use std::sync::RwLock;

pub struct InMemorySecretStore {
    secrets: RwLock<HashMap<String, Secret>>,
}

impl InMemorySecretStore {
    pub fn new(secrets: Vec<Secret>) -> Self {
        let map = secrets
            .into_iter()
            .map(|secret| (format!("{}/{}", secret.namespace, secret.name), secret))
            .collect();
        Self {
            secrets: RwLock::new(map),
        }
    }
}

#[async_trait::async_trait]
impl SecretStore for InMemorySecretStore {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError> {
        let secrets = self.secrets.read().unwrap();
        secrets
            .get(format!("{namespace}/{name}").as_str())
            .cloned()
            .ok_or(SecretStoreError::SecretNotFound(format!(
                "Secret not found: {namespace}/{name}"
            )))
    }

    async fn create_secret(&self, secret: &Secret) -> Result<(), SecretStoreError> {
        let mut secrets = self.secrets.write().unwrap();
        secrets.insert(format!("{}/{}", secret.namespace, secret.name), secret.clone());
        Ok(())
    }
    async fn delete_secret(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError> {
        let mut secrets = self.secrets.write().unwrap();
        secrets.remove(format!("{namespace}/{name}").as_str());
        Ok(())
    }
    async fn set_secret_labels(
        &self,
        _namespace: &str,
        _name: &str,
        _labels: HashMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        Ok(())
    }
}
