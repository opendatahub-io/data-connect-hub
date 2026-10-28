use kube_utils::auth::{AuthError, AuthInfo, KubeAuthClient};

/// RequestIdentity identifies the sender of a request for the secret-access
/// authorization check. Requests reach the rest-service through
/// kube-rbac-proxy, which authenticates the caller itself, strips the
/// `Authorization` header, and injects the username as `x-remote-user`
/// (enabled with `--auth-header-fields-enabled`). The identity is
/// trustworthy only because the proxy also strips client-sent `x-remote-*`
/// headers before injecting its own. The proxy's `x-remote-groups` header,
/// by contrast, is a delimiter-joined string, and Kubernetes group names may
/// themselves contain the separator, so it cannot be decoded losslessly and
/// is never used for authorization.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestIdentity {
    pub username: String,
}

/// resolve_identity picks the identity for a request: the proxy-propagated
/// remote username when present, otherwise `None` so the request is treated
/// as unauthenticated.
pub fn resolve_identity(remote_user: Option<&str>) -> Option<RequestIdentity> {
    let username = remote_user.filter(|username| !username.is_empty())?;
    Some(RequestIdentity {
        username: username.to_string(),
    })
}

/// RequestAuthorizer verifies that the requester behind a request identity may
/// read a named secret in a tenant namespace. The rest-service resolves
/// `credentials_ref` secrets with its own service account, so it must confirm
/// the requesting user could have read the secret themselves before acting on
/// their behalf.
#[async_trait::async_trait]
pub trait RequestAuthorizer: Send + Sync {
    /// check_secret_access reviews the proxy-authenticated identity and
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
        let auth_info = AuthInfo {
            username: identity.username.clone(),
            // Group membership cannot be transported losslessly through
            // the proxy's delimiter-joined header, so the identity is
            // reviewed by username alone. That can only under-approximate
            // the caller's real access: the review matches bindings that
            // name the user directly and never grants anything the
            // caller does not hold.
            groups: Vec::new(),
        };
        self.authorize_secret(&auth_info, tenant_id, secret_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_identity_uses_remote_user() {
        let identity = resolve_identity(Some("proxy-user")).unwrap();

        assert_eq!(
            identity,
            RequestIdentity {
                username: "proxy-user".to_string()
            },
        );
    }

    #[test]
    fn resolve_identity_fails_without_remote_user() {
        assert!(resolve_identity(None).is_none());
    }

    #[test]
    fn resolve_identity_fails_with_empty_remote_user() {
        assert!(resolve_identity(Some("")).is_none());
    }
}
