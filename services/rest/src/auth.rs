use kube_utils::auth::{AuthError, AuthInfo, KubeAuthClient};

/// GROUPS_SEPARATOR joins group names in the proxy's `x-remote-groups`
/// header. It must match kube-rbac-proxy's
/// `--auth-header-groups-field-separator` in the rest-service deployment.
pub const GROUPS_SEPARATOR: char = '|';

/// resolve_identity builds the requester's identity for the secret-access
/// authorization check from the proxy-propagated remote username and groups.
/// It returns `None` when the username is missing or empty so the request is
/// treated as unauthenticated. A missing or empty groups header yields no
/// groups.
///
/// Requests reach the rest-service through kube-rbac-proxy, which
/// authenticates the caller itself, strips the `Authorization` header, and
/// injects the username as `x-remote-user` and the groups as
/// `x-remote-groups` (enabled with `--auth-header-fields-enabled`). The
/// identity is trustworthy only because the proxy overwrites both headers on
/// every authenticated request, so client-sent values never reach the
/// service.
///
/// The groups header joins names with [`GROUPS_SEPARATOR`], so a group name
/// that itself contains the separator is read as several groups. The service
/// cannot tell one such name from several, and no separator is safe for
/// arbitrary identity provider group names. Splitting is therefore safe only
/// while ordinary users cannot choose the names of groups in their own
/// tokens. If the identity provider lets users create and name groups, and a
/// groups prefix (for example `oidc:`) is what keeps those names apart from
/// cluster groups, a user in `oidc:x|secret-readers` is reviewed as a member
/// of `secret-readers`. Administrators must keep the separator out of group
/// names; see section 2.4 of `docs/user-guide/auth.md`.
pub fn resolve_identity(remote_user: Option<&str>, remote_groups: Option<&str>) -> Option<AuthInfo> {
    let username = remote_user.filter(|username| !username.is_empty())?;
    let groups = remote_groups
        .map(|groups| {
            groups
                .split(GROUPS_SEPARATOR)
                .filter(|group| !group.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(AuthInfo {
        username: username.to_string(),
        groups,
    })
}

/// RequestAuthorizer verifies that the requester behind a request identity may
/// read a named secret in a tenant namespace. The rest-service resolves
/// `credentials_ref` secrets with its own service account, so it must confirm
/// the requesting user could have read the secret themselves before acting on
/// their behalf. It exists as a seam so endpoint tests can stub the review.
#[async_trait::async_trait]
pub trait RequestAuthorizer: Send + Sync {
    /// check_secret_access reviews the proxy-authenticated identity and
    /// returns an error if that user cannot read the secret.
    async fn check_secret_access(
        &self,
        identity: &AuthInfo,
        tenant_id: &str,
        secret_name: &str,
    ) -> Result<(), AuthError>;
}

#[async_trait::async_trait]
impl RequestAuthorizer for KubeAuthClient {
    async fn check_secret_access(
        &self,
        identity: &AuthInfo,
        tenant_id: &str,
        secret_name: &str,
    ) -> Result<(), AuthError> {
        self.authorize_secret(identity, tenant_id, secret_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(username: &str, groups: &[&str]) -> AuthInfo {
        AuthInfo {
            username: username.to_string(),
            groups: groups.iter().map(|group| group.to_string()).collect(),
        }
    }

    #[test]
    fn groups_separator_matches_proxy_deployment() {
        // kube-rbac-proxy joins x-remote-groups with the separator configured
        // in the rest-service deployment. If the two drift apart, group names
        // stop splitting and group-granted exports are denied.
        const DEPLOYMENT: &str = include_str!("../../../config/base/rest-service/deployment.yaml");
        const FLAG: &str = "--auth-header-groups-field-separator=";

        let values: Vec<&str> = DEPLOYMENT
            .lines()
            .filter_map(|line| {
                line.trim()
                    .trim_start_matches("- ")
                    .trim_matches('"')
                    .strip_prefix(FLAG)
            })
            .collect();

        assert_eq!(
            values,
            [GROUPS_SEPARATOR.to_string()],
            "{FLAG} in the rest-service deployment must be set once and match GROUPS_SEPARATOR",
        );
    }

    #[test]
    fn resolve_identity_uses_remote_user() {
        assert_eq!(
            resolve_identity(Some("proxy-user"), None),
            Some(identity("proxy-user", &[]))
        );
    }

    #[test]
    fn resolve_identity_fails_without_remote_user() {
        assert!(resolve_identity(None, Some("team-a")).is_none());
    }

    #[test]
    fn resolve_identity_fails_with_empty_remote_user() {
        assert!(resolve_identity(Some(""), Some("team-a")).is_none());
    }

    #[test]
    fn resolve_identity_empty_groups_header_yields_no_groups() {
        assert_eq!(resolve_identity(Some("alice"), Some("")), Some(identity("alice", &[])));
    }

    #[test]
    fn resolve_identity_parses_single_group() {
        assert_eq!(
            resolve_identity(Some("alice"), Some("system:authenticated")),
            Some(identity("alice", &["system:authenticated"])),
        );
    }

    #[test]
    fn resolve_identity_parses_multiple_groups() {
        assert_eq!(
            resolve_identity(
                Some("system:serviceaccount:team-a:runner"),
                Some("system:serviceaccounts|system:serviceaccounts:team-a|system:authenticated"),
            ),
            Some(identity(
                "system:serviceaccount:team-a:runner",
                &[
                    "system:serviceaccounts",
                    "system:serviceaccounts:team-a",
                    "system:authenticated"
                ],
            )),
        );
    }

    #[test]
    fn resolve_identity_skips_empty_group_segments() {
        assert_eq!(
            resolve_identity(Some("alice"), Some("|team-a||team-b|")),
            Some(identity("alice", &["team-a", "team-b"])),
        );
    }

    #[test]
    fn resolve_identity_keeps_comma_inside_group_name() {
        // Only GROUPS_SEPARATOR splits names; other punctuation such as the
        // commas in LDAP-style names stays part of a single group.
        assert_eq!(
            resolve_identity(Some("alice"), Some("cn=readers,ou=groups")),
            Some(identity("alice", &["cn=readers,ou=groups"])),
        );
    }
}
