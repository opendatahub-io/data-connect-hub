use k8s_openapi::api::authentication::v1::{TokenReview, TokenReviewSpec};
use k8s_openapi::api::authorization::v1::{ResourceAttributes, SubjectAccessReview, SubjectAccessReviewSpec};
use kube::api::PostParams;
use kube::{Api, Client};
use moka::future::Cache;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

fn hash_token(token: &str) -> i64 {
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish() as i64
}

#[derive(Error, Debug, Clone)]
pub enum AuthError {
    #[error("Unauthenticated: {0}")]
    Unauthenticated(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("TokenReview API call failed: {0}")]
    TokenReviewError(String),
    #[error("SubjectAccessReview API call failed: {0}")]
    AccessReviewError(String),
    #[error("Internal auth error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct AuthInfo {
    pub username: String,
    pub groups: Vec<String>,
}

pub struct KubeAuthClient {
    client: Client,
    token_review_audiences: Vec<String>,
    token_cache: Cache<i64, Result<AuthInfo, AuthError>>,
    sar_cache: Cache<String, bool>,
}

const API_GROUP: &str = "dataconnecthub.opendatahub.io";
const RESOURCE: &str = "data-connections";

impl KubeAuthClient {
    pub fn new(client: Client, cache_ttl: Duration, token_review_audiences: Vec<String>) -> Self {
        Self {
            client,
            token_review_audiences,
            token_cache: Cache::builder().time_to_live(cache_ttl).max_capacity(10_000).build(),
            sar_cache: Cache::builder().time_to_live(cache_ttl).max_capacity(10_000).build(),
        }
    }

    pub async fn try_default(cache_ttl: Duration, token_review_audiences: Vec<String>) -> Result<Self, kube::Error> {
        let client = Client::try_default().await?;
        Ok(Self::new(client, cache_ttl, token_review_audiences))
    }

    pub async fn authenticate(&self, token: &str) -> Result<AuthInfo, AuthError> {
        let client = self.client.clone();
        let token_owned = token.to_string();
        let audiences = self.token_review_audiences.clone();
        let cache_key = hash_token(token);

        self.token_cache
            .try_get_with(cache_key, async move {
                debug!("Performing TokenReview");
                let review = TokenReview {
                    spec: TokenReviewSpec {
                        token: token_owned,
                        audiences: Some(audiences.clone()),
                    },
                    ..Default::default()
                };

                let api: Api<TokenReview> = Api::all(client);
                let result = api
                    .create(&PostParams::default(), &review)
                    .await
                    .map_err(|e| AuthError::TokenReviewError(e.to_string()))?;

                let status = result
                    .status
                    .ok_or(AuthError::Internal("TokenReview returned no status".into()))?;

                if !status.authenticated.unwrap_or(false) {
                    let reason = status.error.unwrap_or_else(|| "token not authenticated".into());
                    warn!("TokenReview rejected: {reason}");
                    return Ok(Err(AuthError::Unauthenticated(reason)));
                }

                if !has_compatible_audience(&audiences, status.audiences.as_deref()) {
                    warn!(
                        "TokenReview rejected: no compatible audience returned (requested={audiences:?}, returned={:?})",
                        status.audiences
                    );
                    return Ok(Err(AuthError::Unauthenticated("token audience mismatch".into())));
                }

                let user = status
                    .user
                    .ok_or(AuthError::Internal("TokenReview returned no user info".into()))?;

                let info = AuthInfo {
                    username: user.username.unwrap_or_default(),
                    groups: user.groups.unwrap_or_default(),
                };
                debug!("TokenReview authenticated user: {}", info.username);
                Ok(Ok(info))
            })
            .await
            .map_err(|e: Arc<AuthError>| e.as_ref().clone())?
    }

    pub async fn authorize(&self, auth_info: &AuthInfo, tenant_id: &str, verb: &str) -> Result<(), AuthError> {
        let key = format!(
            "conn:{}:{:?}:{}:{}",
            auth_info.username, auth_info.groups, tenant_id, verb
        );
        let client = self.client.clone();
        let username = auth_info.username.clone();
        let groups = auth_info.groups.clone();
        let ns = tenant_id.to_string();
        let v = verb.to_string();

        let allowed = self
            .sar_cache
            .try_get_with(key, async move {
                debug!("Performing SubjectAccessReview for user={username} namespace={ns} verb={v}");
                let sar = SubjectAccessReview {
                    spec: SubjectAccessReviewSpec {
                        user: Some(username),
                        groups: Some(groups),
                        resource_attributes: Some(ResourceAttributes {
                            namespace: Some(ns),
                            verb: Some(v),
                            group: Some(API_GROUP.into()),
                            resource: Some(RESOURCE.into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let api: Api<SubjectAccessReview> = Api::all(client);
                let result = api
                    .create(&PostParams::default(), &sar)
                    .await
                    .map_err(|e| AuthError::AccessReviewError(e.to_string()))?;

                let status = result
                    .status
                    .ok_or_else(|| AuthError::Internal("SAR returned no status".into()))?;

                debug!("SubjectAccessReview result: allowed={}", status.allowed);
                Ok(status.allowed)
            })
            .await
            .map_err(|e: Arc<AuthError>| e.as_ref().clone())?;

        if allowed {
            Ok(())
        } else {
            Err(AuthError::Forbidden(format!(
                "access denied for {RESOURCE} in namespace {tenant_id}"
            )))
        }
    }

    /// authorize_secret checks that the requesting user (not the service's own
    /// identity) may read the named secret in the tenant namespace. It exists
    /// so services that resolve a connection's `credentials_ref` on a user's
    /// behalf cannot act as a confused deputy for secrets the user cannot
    /// read directly.
    pub async fn authorize_secret(
        &self,
        auth_info: &AuthInfo,
        tenant_id: &str,
        secret_name: &str,
    ) -> Result<(), AuthError> {
        let key = secret_cache_key(auth_info, tenant_id, secret_name);
        let spec = secret_review_spec(auth_info, tenant_id, secret_name);
        let client = self.client.clone();

        let allowed = self
            .sar_cache
            .try_get_with(key, async move {
                let secret_name = spec
                    .resource_attributes
                    .as_ref()
                    .and_then(|a| a.name.clone())
                    .unwrap_or_default();
                debug!(
                    "Performing SubjectAccessReview for user={} namespace={} secret={}",
                    spec.user.as_deref().unwrap_or_default(),
                    spec.resource_attributes
                        .as_ref()
                        .and_then(|a| a.namespace.clone())
                        .unwrap_or_default(),
                    secret_name
                );
                let sar = SubjectAccessReview {
                    spec,
                    ..Default::default()
                };

                let api: Api<SubjectAccessReview> = Api::all(client);
                let result = api
                    .create(&PostParams::default(), &sar)
                    .await
                    .map_err(|e| AuthError::AccessReviewError(e.to_string()))?;

                let status = result
                    .status
                    .ok_or_else(|| AuthError::Internal("SAR returned no status".into()))?;

                debug!("SubjectAccessReview result: allowed={}", status.allowed);
                Ok(status.allowed)
            })
            .await
            .map_err(|e: Arc<AuthError>| e.as_ref().clone())?;

        if allowed {
            Ok(())
        } else {
            Err(AuthError::Forbidden(format!(
                "access denied for secret {secret_name} in namespace {tenant_id}"
            )))
        }
    }
}

fn secret_cache_key(auth_info: &AuthInfo, tenant_id: &str, secret_name: &str) -> String {
    format!(
        "secret:{}:{:?}:{}:secrets:get:{}",
        auth_info.username, auth_info.groups, tenant_id, secret_name
    )
}

fn secret_review_spec(auth_info: &AuthInfo, tenant_id: &str, secret_name: &str) -> SubjectAccessReviewSpec {
    SubjectAccessReviewSpec {
        user: Some(auth_info.username.clone()),
        groups: Some(auth_info.groups.clone()),
        resource_attributes: Some(ResourceAttributes {
            namespace: Some(tenant_id.to_string()),
            verb: Some("get".into()),
            group: Some("".into()),
            resource: Some("secrets".into()),
            name: Some(secret_name.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn has_compatible_audience(requested: &[String], returned: Option<&[String]>) -> bool {
    if requested.is_empty() {
        return true;
    }

    returned
        .unwrap_or_default()
        .iter()
        .any(|audience| requested.iter().any(|candidate| candidate == audience))
}

#[cfg(test)]
mod tests {
    use super::{AuthInfo, has_compatible_audience, secret_review_spec};

    fn auth_info() -> AuthInfo {
        AuthInfo {
            username: "system:serviceaccount:tenant-a:dch-test-user".to_string(),
            groups: vec!["system:serviceaccounts".to_string()],
        }
    }

    #[test]
    fn rejects_authenticated_response_without_returned_audience() {
        let requested = vec!["https://kubernetes.default.svc".to_string()];

        assert!(!has_compatible_audience(&requested, None));
    }

    #[test]
    fn rejects_authenticated_response_without_matching_audience() {
        let requested = vec!["https://kubernetes.default.svc".to_string()];
        let returned = vec!["https://other-audience".to_string()];

        assert!(!has_compatible_audience(&requested, Some(returned.as_slice())));
    }

    #[test]
    fn accepts_authenticated_response_with_matching_audience() {
        let requested = vec!["https://kubernetes.default.svc".to_string()];
        let returned = vec![
            "https://other-audience".to_string(),
            "https://kubernetes.default.svc".to_string(),
        ];

        assert!(has_compatible_audience(&requested, Some(returned.as_slice())));
    }

    #[test]
    fn secret_review_spec_targets_named_secret_in_core_group() {
        let spec = secret_review_spec(&auth_info(), "tenant-a", "db-secret");

        let attrs = spec.resource_attributes.expect("resource attributes");
        assert_eq!(attrs.namespace.as_deref(), Some("tenant-a"));
        assert_eq!(attrs.verb.as_deref(), Some("get"));
        assert_eq!(attrs.group.as_deref(), Some(""));
        assert_eq!(attrs.resource.as_deref(), Some("secrets"));
        assert_eq!(attrs.name.as_deref(), Some("db-secret"));
        assert_eq!(
            spec.user.as_deref(),
            Some("system:serviceaccount:tenant-a:dch-test-user")
        );
        assert_eq!(spec.groups, Some(vec!["system:serviceaccounts".to_string()]));
    }

    #[test]
    fn secret_review_spec_scopes_cache_key_by_secret_name() {
        let first = super::secret_cache_key(&auth_info(), "tenant-a", "db-secret");
        let second = super::secret_cache_key(&auth_info(), "tenant-a", "other-secret");
        let other_tenant = super::secret_cache_key(&auth_info(), "tenant-b", "db-secret");

        assert_ne!(first, second);
        assert_ne!(first, other_tenant);
    }

    #[test]
    fn secret_cache_key_is_namespaced_apart_from_tenant_cache_keys() {
        // The conn-level authorize() key is "conn:{user}:{groups}:{tenant}:{verb}";
        // the two key families share one cache, so they must be structurally
        // disjoint even for identical users, tenants, and verbs.
        let secret_key = super::secret_cache_key(&auth_info(), "tenant-a", "get");
        let conn_key = format!(
            "conn:{}:{:?}:{}:{}",
            auth_info().username,
            auth_info().groups,
            "tenant-a",
            "get"
        );

        assert!(secret_key.starts_with("secret:"));
        assert_ne!(secret_key, conn_key);
    }
}
