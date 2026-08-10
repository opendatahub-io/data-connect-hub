use k8s_openapi::api::authentication::v1::{TokenReview, TokenReviewSpec};
use k8s_openapi::api::authorization::v1::{ResourceAttributes, SubjectAccessReview, SubjectAccessReviewSpec};
use kube::api::PostParams;
use kube::{Api, Client};
use moka::future::Cache;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Error, Debug, Clone)]
pub enum AuthError {
    #[error("Unauthenticated: {0}")]
    Unauthenticated(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
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
    token_cache: Cache<String, Result<AuthInfo, AuthError>>,
    sar_cache: Cache<String, bool>,
}

const API_GROUP: &str = "dataconnecthub.opendatahub.io";
const RESOURCE: &str = "data-connections";

impl KubeAuthClient {
    pub fn new(client: Client, cache_ttl: Duration) -> Self {
        Self {
            client,
            token_cache: Cache::builder().time_to_live(cache_ttl).max_capacity(10_000).build(),
            sar_cache: Cache::builder().time_to_live(cache_ttl).max_capacity(10_000).build(),
        }
    }

    pub async fn try_default(cache_ttl: Duration) -> Result<Self, kube::Error> {
        let client = Client::try_default().await?;
        Ok(Self::new(client, cache_ttl))
    }

    pub async fn authenticate(&self, token: &str) -> Result<AuthInfo, AuthError> {
        let client = self.client.clone();
        let token_owned = token.to_string();
        let cache_key = hash_token(token);

        self.token_cache
            .try_get_with(cache_key, async move {
                debug!("Performing TokenReview");
                let review = TokenReview {
                    spec: TokenReviewSpec {
                        token: Some(token_owned),
                        audiences: Some(vec!["https://kubernetes.default.svc".to_string()]),
                    },
                    ..Default::default()
                };

                let api: Api<TokenReview> = Api::all(client);
                let result = api
                    .create(&PostParams::default(), &review)
                    .await
                    .map_err(|e| AuthError::Internal(format!("TokenReview API call failed: {e}")))?;

                let status = result
                    .status
                    .ok_or(AuthError::Internal("TokenReview returned no status".into()))?;

                if !status.authenticated.unwrap_or(false) {
                    let reason = status.error.unwrap_or_else(|| "token not authenticated".into());
                    warn!("TokenReview rejected: {reason}");
                    return Ok(Err(AuthError::Unauthenticated(reason)));
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
        let key = format!("{}:{:?}:{}:{}", auth_info.username, auth_info.groups, tenant_id, verb);
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
                    .map_err(|e| AuthError::Internal(format!("SubjectAccessReview API call failed: {e}")))?;

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
}
