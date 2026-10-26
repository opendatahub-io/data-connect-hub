use kube_utils::auth::{AuthError, AuthInfo, KubeAuthClient};

/// RequestIdentity identifies the sender of a request for the secret-access
/// authorization check. Behind kube-rbac-proxy the bearer token never reaches
/// this service (the k8s.io/apiserver authenticator deletes the Authorization
/// header during proxy authentication), so identity can also arrive as the
/// proxy-injected x-remote-user/x-remote-groups headers.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestIdentity {
    BearerToken(String),
    RemoteUser { username: String, groups: Vec<String> },
}

/// resolve_identity picks the identity for a request: a bearer token when one
/// is present (it is self-authenticating via TokenReview), otherwise the
/// proxy-propagated remote user, otherwise an error. The bearer token takes
/// priority because it is verified against the API server, while the remote
/// headers are only trustworthy when the request arrived through the proxy.
pub fn resolve_identity(
    auth_token: Option<&str>,
    remote_user: Option<&str>,
    remote_groups: Option<&str>,
) -> Result<RequestIdentity, AuthError> {
    if let Some(token) = auth_token.filter(|token| !token.is_empty()) {
        return Ok(RequestIdentity::BearerToken(token.to_string()));
    }
    if let Some(username) = remote_user.filter(|username| !username.is_empty()) {
        let groups = remote_groups
            .map(|joined| joined.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        return Ok(RequestIdentity::RemoteUser {
            username: username.to_string(),
            groups,
        });
    }
    Err(AuthError::Unauthenticated(
        "no authenticated identity on request".to_string(),
    ))
}

/// RequestAuthorizer verifies that the requester behind a request identity may
/// read a named secret in a tenant namespace. The rest-service resolves
/// `credentials_ref` secrets with its own service account, so it must confirm
/// the requesting user could have read the secret themselves before acting on
/// their behalf.
#[async_trait::async_trait]
pub trait RequestAuthorizer: Send + Sync {
    /// check_secret_access resolves the identity to an authenticated user and
    /// returns an error if that user cannot read the secret.
    async fn check_secret_access(
        &self,
        identity: &RequestIdentity,
        tenant_id: &str,
        secret_name: &str,
    ) -> Result<(), AuthError>;
}

#[async_trait::async_trait]
impl RequestAuthorizer for KubeAuthClient {
    async fn check_secret_access(
        &self,
        identity: &RequestIdentity,
        tenant_id: &str,
        secret_name: &str,
    ) -> Result<(), AuthError> {
        let auth_info = match identity {
            RequestIdentity::BearerToken(token) => self.authenticate(token).await?,
            RequestIdentity::RemoteUser { username, groups } => AuthInfo {
                username: username.clone(),
                groups: groups.clone(),
            },
        };
        self.authorize_secret(&auth_info, tenant_id, secret_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_identity_prefers_bearer_token_over_remote_headers() {
        let identity = resolve_identity(Some("some-token"), Some("proxy-user"), Some("group-a,group-b")).unwrap();

        assert_eq!(identity, RequestIdentity::BearerToken("some-token".to_string()));
    }

    #[test]
    fn resolve_identity_uses_remote_user_when_no_token() {
        let identity = resolve_identity(None, Some("proxy-user"), Some("group-a,group-b")).unwrap();

        assert_eq!(
            identity,
            RequestIdentity::RemoteUser {
                username: "proxy-user".to_string(),
                groups: vec!["group-a".to_string(), "group-b".to_string()],
            },
        );
    }

    #[test]
    fn resolve_identity_without_groups_yields_empty_group_list() {
        let identity = resolve_identity(None, Some("proxy-user"), None).unwrap();

        assert_eq!(
            identity,
            RequestIdentity::RemoteUser {
                username: "proxy-user".to_string(),
                groups: vec![]
            },
        );
    }

    #[test]
    fn resolve_identity_fails_without_token_or_remote_user() {
        let result = resolve_identity(None, None, None);

        assert!(matches!(result, Err(AuthError::Unauthenticated(_))));
    }

    #[test]
    fn resolve_identity_fails_with_empty_remote_user() {
        let result = resolve_identity(None, Some(""), Some("group-a"));

        assert!(matches!(result, Err(AuthError::Unauthenticated(_))));
    }
}
