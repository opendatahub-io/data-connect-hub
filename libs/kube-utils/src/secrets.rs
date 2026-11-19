use commons::api::errors::{CreateSecretErrorKind, SecretStoreError};
use commons::api::secret::Secret;
use commons::api::storage::SecretStore;
use k8s_openapi::api::core::v1::Secret as K8sSecret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, Patch, PatchParams, PostParams};
use kube::core::request;
use kube::{Api, Client};
use std::collections::HashMap;
use tracing::error;

pub struct KubeSecretStore {
    client: Client,
}

impl KubeSecretStore {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn try_default() -> Result<Self, kube::Error> {
        let client = Client::try_default().await?;
        Ok(Self::new(client))
    }
}

#[async_trait::async_trait]
impl SecretStore for KubeSecretStore {
    #[tracing::instrument(
        skip_all,
        fields(
        k8s.api.operation = "get",
        k8s.resource.kind = "Secret",
        k8s.namespace.name = %namespace,
        k8s.secret.name = %name,
        )
    )]
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError> {
        let api: Api<K8sSecret> = Api::namespaced(self.client.clone(), namespace);
        let k8s_secret = api.get(name).await.map_err(|e| {
            error!("failed to get secret {namespace}/{name}: {e}");
            SecretStoreError::SecretNotFound("Failed to obtain credentials".to_string())
        })?;
        let properties = extract_properties(&k8s_secret);
        Ok(Secret {
            name: name.to_string(),
            namespace: namespace.to_string(),
            properties,
            labels: Some(k8s_secret.metadata.labels.unwrap_or_default().into_iter().collect()),
            annotations: Some(
                k8s_secret
                    .metadata
                    .annotations
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            ),
        })
    }

    #[tracing::instrument(
        skip_all,
        fields(
        k8s.api.operation = if overwrite { "patch" } else { "create" },
        k8s.resource.kind = "Secret",
        k8s.namespace.name = %secret.namespace,
        k8s.secret.name = %secret.name,
        )
    )]
    async fn create_secret(&self, secret: &Secret, overwrite: bool) -> Result<(), SecretStoreError> {
        let ns = &secret.namespace;

        let api: Api<K8sSecret> = Api::namespaced(self.client.clone(), ns);

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
            error!("failed to create secret {ns}/{}: {e}", secret.name);
            create_secret_error(&secret.name, ns, &e)
        })?;

        Ok(())
    }

    #[tracing::instrument(
        skip_all,
        fields(
        k8s.api.operation = "delete",
        k8s.resource.kind = "Secret",
        k8s.namespace.name = %namespace,
        k8s.secret.name = %name,
        )
    )]
    async fn delete_secret(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError> {
        let api: Api<K8sSecret> = Api::namespaced(self.client.clone(), namespace);
        api.delete(name, &DeleteParams::default()).await.map_err(|e| {
            error!("failed to delete secret {namespace}/{name}: {e}");
            SecretStoreError::SecretNotFound(format!("{namespace}/{name}"))
        })?;

        Ok(())
    }

    #[tracing::instrument(
        skip_all,
        fields(
        k8s.api.operation = "patch",
        k8s.resource.kind = "Secret",
        k8s.namespace.name = %namespace,
        k8s.secret.name = %name,
        )
    )]
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

/// Builds the caller-facing error for a failed secret write.
///
/// The Kubernetes error text is passed through as is: it is returned only to
/// the caller, who supplied the credentials (create) or can already read them
/// (export), and it names the exact cause, such as the denied ServiceAccount
/// or the admission webhook that rejected the request.
///
/// The Kubernetes status code is not passed through, because it describes the
/// DCH service account's request rather than the caller's: a 401 or 403 there
/// is a DCH or cluster configuration problem, not a caller credential problem.
fn create_secret_error(name: &str, namespace: &str, err: &kube::Error) -> SecretStoreError {
    let reason = match err {
        kube::Error::Api(status) if !status.message.is_empty() => status.message.clone(),
        kube::Error::Api(status) if !status.reason.is_empty() => status.reason.clone(),
        kube::Error::Api(status) => format!("Kubernetes API request failed with HTTP {}", status.code),
        kube::Error::BuildRequest(request::Error::BuildRequest(e)) => {
            format!("the secret name or namespace is not valid in a request URI: {e}")
        },
        other => other.to_string(),
    };
    let kind = match err {
        kube::Error::Api(status) if status.is_already_exists() || status.is_conflict() || status.code == 409 => {
            CreateSecretErrorKind::Conflict
        },
        kube::Error::Api(status) if status.is_invalid() || status.code == 422 => CreateSecretErrorKind::Invalid,
        // The namespace comes from the caller's X-Tenant-Id; a Secret write
        // only returns 404 when that namespace does not exist.
        kube::Error::Api(status) if status.is_not_found() || status.code == 404 => CreateSecretErrorKind::Invalid,
        kube::Error::Api(status) if status.code == 429 || status.code >= 500 => CreateSecretErrorKind::Unavailable,
        // The request URI is built from the caller-supplied namespace and
        // secret name, so a URI the client cannot build is the caller's error.
        // The patch path hits this before Kubernetes can validate the name.
        kube::Error::BuildRequest(request::Error::BuildRequest(_)) => CreateSecretErrorKind::Invalid,
        kube::Error::HyperError(_) | kube::Error::Service(_) => CreateSecretErrorKind::Unavailable,
        _ => CreateSecretErrorKind::Internal,
    };
    SecretStoreError::CannotCreateSecret {
        name: name.to_string(),
        namespace: namespace.to_string(),
        reason,
        kind,
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
    use kube::core::Status;

    fn api_error(code: u16, reason: &str, message: &str) -> kube::Error {
        kube::Error::Api(Status::failure(message, reason).with_code(code).boxed())
    }

    fn create_reason(err: kube::Error) -> (String, CreateSecretErrorKind) {
        match create_secret_error("my-secret", "team-alpha", &err) {
            SecretStoreError::CannotCreateSecret {
                name,
                namespace,
                reason,
                kind,
            } => {
                assert_eq!(name, "my-secret");
                assert_eq!(namespace, "team-alpha");
                (reason, kind)
            },
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn test_create_secret_error_rbac_forbidden_is_internal() {
        let message = "secrets is forbidden: User \"system:serviceaccount:dch:rest\" cannot create resource \"secrets\" in API group \"\" in the namespace \"team-alpha\"";
        assert_eq!(
            create_reason(api_error(403, "Forbidden", message)),
            (message.to_string(), CreateSecretErrorKind::Internal)
        );
    }

    #[test]
    fn test_create_secret_error_unauthorized_is_internal() {
        let (_, kind) = create_reason(api_error(401, "Unauthorized", "Unauthorized"));
        assert_eq!(kind, CreateSecretErrorKind::Internal);
    }

    #[test]
    fn test_create_secret_error_already_exists_is_conflict() {
        let message = "secrets \"my-secret\" already exists";
        assert_eq!(
            create_reason(api_error(409, "AlreadyExists", message)),
            (message.to_string(), CreateSecretErrorKind::Conflict)
        );
    }

    #[test]
    fn test_create_secret_error_apply_conflict_is_conflict() {
        let (_, kind) = create_reason(api_error(409, "Conflict", "Apply failed with 1 conflict"));
        assert_eq!(kind, CreateSecretErrorKind::Conflict);
    }

    #[test]
    fn test_create_secret_error_invalid_is_invalid() {
        let (_, kind) = create_reason(api_error(422, "Invalid", "Secret \"My_Secret\" is invalid"));
        assert_eq!(kind, CreateSecretErrorKind::Invalid);
    }

    #[test]
    fn test_create_secret_error_throttled_is_unavailable() {
        let (_, kind) = create_reason(api_error(
            429,
            "TooManyRequests",
            "Too many requests, please try again later.",
        ));
        assert_eq!(kind, CreateSecretErrorKind::Unavailable);
    }

    #[test]
    fn test_create_secret_error_unbuildable_uri_is_invalid() {
        let err = request::Request::new("/api/v1/namespaces/team-alpha/secrets")
            .patch(
                "bad name",
                &PatchParams::apply("data-connect-hub"),
                &Patch::Apply(&K8sSecret::default()),
            )
            .unwrap_err();
        let (reason, kind) = create_reason(kube::Error::BuildRequest(err));
        assert_eq!(kind, CreateSecretErrorKind::Invalid);
        assert!(
            reason.starts_with("the secret name or namespace is not valid in a request URI: "),
            "{reason}"
        );
    }

    #[test]
    fn test_create_secret_error_server_error_is_unavailable() {
        let (_, kind) = create_reason(api_error(500, "InternalError", "etcd leader changed"));
        assert_eq!(kind, CreateSecretErrorKind::Unavailable);
    }

    #[test]
    fn test_create_secret_error_missing_namespace_is_invalid() {
        let message = "namespaces \"team-alpha\" not found";
        assert_eq!(
            create_reason(api_error(404, "NotFound", message)),
            (message.to_string(), CreateSecretErrorKind::Invalid)
        );
    }

    #[test]
    fn test_create_secret_error_empty_api_message_falls_back_to_reason() {
        let (reason, _) = create_reason(api_error(500, "InternalError", ""));
        assert_eq!(reason, "InternalError");
    }

    #[test]
    fn test_create_secret_error_empty_api_message_and_reason_falls_back_to_code() {
        let (reason, _) = create_reason(api_error(500, "", ""));
        assert_eq!(reason, "Kubernetes API request failed with HTTP 500");
    }

    #[test]
    fn test_create_secret_error_transport_error_is_unavailable() {
        let err = kube::Error::Service(Box::new(std::io::Error::other("connection refused")));
        assert_eq!(
            create_reason(err),
            (
                "ServiceError: connection refused".to_string(),
                CreateSecretErrorKind::Unavailable
            )
        );
    }

    #[test]
    fn test_create_secret_error_display() {
        let err = create_secret_error(
            "my-secret",
            "team-alpha",
            &api_error(409, "AlreadyExists", "secrets \"my-secret\" already exists"),
        );
        assert_eq!(
            err.to_string(),
            "Cannot create secret 'my-secret' in namespace 'team-alpha': secrets \"my-secret\" already exists"
        );
    }

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
