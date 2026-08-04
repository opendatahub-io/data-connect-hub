use commons::api::connections::{Secret, SecretStore};
use commons::api::errors::SecretStoreError;
use k8s_openapi::api::core::v1::Secret as K8sSecret;
use kube::{Api, Client};
use moka::future::Cache;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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
                let k8s_secret = api
                    .get(&n)
                    .await
                    .map_err(|_| SecretStoreError::SecretNotFound("Failed to obtain credentials".to_string()))?;
                let properties = extract_properties(&k8s_secret);
                Ok(Secret {
                    name: n,
                    namespace: ns,
                    properties,
                })
            })
            .await
            .map_err(|e: Arc<SecretStoreError>| match e.as_ref() {
                SecretStoreError::SecretNotFound(msg) => SecretStoreError::SecretNotFound(msg.clone()),
                SecretStoreError::Forbidden(msg) => SecretStoreError::Forbidden(msg.clone()),
            })
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
