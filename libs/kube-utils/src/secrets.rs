use commons::api::errors::SecretStoreError;
use commons::api::secret::Secret;
use commons::api::storage::SecretStore;
use k8s_openapi::api::core::v1::Secret as K8sSecret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use moka::future::Cache;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::error;

pub struct KubeSecretStore {
    client: Client,
    cache: Cache<String, Secret>,
}

impl KubeSecretStore {
    pub fn new(client: Client, cache_ttl: Duration) -> Self {
        Self {
            client,
            cache: Cache::builder().time_to_live(cache_ttl).build(),
        }
    }

    pub async fn try_default(cache_ttl: Duration) -> Result<Self, kube::Error> {
        let client = Client::try_default().await?;
        Ok(Self::new(client, cache_ttl))
    }
}

#[async_trait::async_trait]
impl SecretStore for KubeSecretStore {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError> {
        let key = format!("{namespace}/{name}");
        let client = self.client.clone();
        let ns = namespace.to_string();
        let n = name.to_string();

        self.cache
            .try_get_with(key, async move {
                let api: Api<K8sSecret> = Api::namespaced(client, &ns);
                let k8s_secret = api.get(&n).await.map_err(|e| {
                    error!("failed to get secret {ns}/{n}: {e}");
                    SecretStoreError::SecretNotFound("Failed to obtain credentials".to_string())
                })?;
                let properties = extract_properties(&k8s_secret);
                Ok(Secret {
                    name: n,
                    namespace: ns,
                    properties,
                    labels: Some(
                        k8s_secret
                            .metadata
                            .labels
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                    ),
                    annotations: Some(
                        k8s_secret
                            .metadata
                            .annotations
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                    ),
                })
            })
            .await
            .map_err(|e: Arc<SecretStoreError>| e.as_ref().clone())
    }

    async fn create_secret(&self, secret: &Secret, overwrite: bool) -> Result<(), SecretStoreError> {
        let ns = Arc::new(secret.namespace.clone());

        let api: Api<K8sSecret> = Api::namespaced(self.client.clone(), &ns);

        let labels = secret.labels.clone().map(|l| l.into_iter().collect());
        let annotations = secret.annotations.clone().map(|a| a.into_iter().collect());

        let k8s_secret = K8sSecret {
            metadata: ObjectMeta {
                name: Some(secret.name.clone()),
                namespace: Some(ns.to_string()),
                labels,
                annotations,
                ..Default::default()
            },
            string_data: Some(secret.properties.clone().into_iter().collect()),
            ..Default::default()
        };

        if overwrite {
            api.patch(
                &secret.name,
                &PatchParams::apply("data-connect-hub"),
                &Patch::Apply(&k8s_secret),
            )
            .await
        } else {
            api.create(&PostParams::default(), &k8s_secret).await
        }
        .map_err(|e| {
            error!("Failed to create secret {}: {}", secret.name, e);
            SecretStoreError::CannotCreateSecret(secret.name.clone())
        })?;

        Ok(())
    }

    async fn delete_secret(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError> {
        let api: Api<K8sSecret> = Api::namespaced(self.client.clone(), namespace);
        api.delete(name, &DeleteParams::default()).await.map_err(|e| {
            error!("failed to delete secret {namespace}/{name}: {e}");
            SecretStoreError::SecretNotFound(format!("{namespace}/{name}"))
        })?;

        let key = format!("{namespace}/{name}");
        self.cache.invalidate(&key).await;

        Ok(())
    }

    async fn set_secret_labels(
        &self,
        namespace: &str,
        name: &str,
        labels: HashMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        let api: Api<K8sSecret> = Api::namespaced(self.client.clone(), namespace);
        let patch = serde_json::json!({
            "metadata": {
                "labels": labels
            }
        });
        api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .map_err(|e| {
                error!("failed to set labels on secret {namespace}/{name}: {e}");
                SecretStoreError::SecretNotFound(format!("{namespace}/{name}"))
            })?;

        Ok(())
    }
}

fn extract_properties(k8s_secret: &K8sSecret) -> HashMap<String, String> {
    k8s_secret
        .data
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(key, value)| String::from_utf8(value.0).ok().map(|v| (key, v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::ByteString;

    fn k8s_secret_with_data(data: Vec<(&str, &[u8])>) -> K8sSecret {
        K8sSecret {
            data: Some(
                data.into_iter()
                    .map(|(k, v)| (k.to_string(), ByteString(v.to_vec())))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn test_extract_properties_from_valid_utf8() {
        let k8s = k8s_secret_with_data(vec![
            ("url", b"postgresql://localhost:5432/mydb"),
            ("password", b"s3cret"),
        ]);

        let props = extract_properties(&k8s);
        assert_eq!(props.len(), 2);
        assert_eq!(props["url"], "postgresql://localhost:5432/mydb");
        assert_eq!(props["password"], "s3cret");
    }

    #[test]
    fn test_extract_properties_skips_invalid_utf8() {
        let k8s = k8s_secret_with_data(vec![("valid", b"hello"), ("binary", &[0xff, 0xfe, 0xfd])]);

        let props = extract_properties(&k8s);
        assert_eq!(props.len(), 1);
        assert_eq!(props["valid"], "hello");
        assert!(!props.contains_key("binary"));
    }

    #[test]
    fn test_extract_properties_empty_data() {
        let k8s = K8sSecret {
            data: None,
            ..Default::default()
        };

        let props = extract_properties(&k8s);
        assert!(props.is_empty());
    }

    #[test]
    fn test_extract_properties_empty_value() {
        let k8s = k8s_secret_with_data(vec![("key", b"")]);

        let props = extract_properties(&k8s);
        assert_eq!(props.len(), 1);
        assert_eq!(props["key"], "");
    }
}
