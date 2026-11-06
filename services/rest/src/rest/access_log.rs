use actix_web::http::Method;
use std::time::Duration;

/// AuditConfig controls whether REST audit records are emitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuditConfig {
    /// enabled indicates whether request audit events should be emitted.
    pub enabled: bool,
}

impl AuditConfig {
    /// from_env reads the controller-provided `AUDIT_ENABLED` setting.
    pub fn from_env() -> Self {
        Self::from_value(std::env::var("AUDIT_ENABLED").ok().as_deref())
    }

    fn from_value(value: Option<&str>) -> Self {
        Self {
            enabled: value.is_some_and(|value| value.eq_ignore_ascii_case("true")),
        }
    }
}

/// RequestId uniquely identifies one inbound REST request.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// AuditQuery carries what the request actually read, set by a handler so the
/// audit middleware can record it in the `audit.query` field. For binary
/// downloads this is the path of the file being downloaded.
#[derive(Clone, Debug)]
pub struct AuditQuery(pub String);

/// AuditError contains the safe error details recorded in an audit event.
#[derive(Clone, Debug)]
pub struct AuditError {
    /// code is the stable REST error code.
    pub code: String,
    /// message is a sanitized description safe for the audit stream.
    pub message: String,
}

/// resource_action_for maps an HTTP operation to its documented audit operation.
pub fn resource_action_for(method: &Method, route: &str) -> Option<(&'static str, &'static str)> {
    let route = route.strip_prefix("/api/v1alpha1/data")?;
    match (method, route) {
        (&Method::GET, "/connection-types") => Some(("connection_type", "list")),
        (&Method::POST, "/connection-types") => Some(("connection_type", "create")),
        (&Method::GET, "/connection-types/{id}") => Some(("connection_type", "get")),
        (&Method::PATCH, "/connection-types/{id}") => Some(("connection_type", "update")),
        (&Method::DELETE, "/connection-types/{id}") => Some(("connection_type", "delete")),
        (&Method::GET, "/connections") => Some(("connection", "list")),
        (&Method::POST, "/connections") => Some(("connection", "create")),
        (&Method::GET, "/connections/{id}") => Some(("connection", "get")),
        (&Method::PATCH, "/connections/{id}") => Some(("connection", "update")),
        (&Method::DELETE, "/connections/{id}") => Some(("connection", "delete")),
        (&Method::PUT, "/connections/{id}/exports/secrets/{secret_name}") => Some(("connection_secret", "export")),
        (&Method::POST, "/connections/{id}/readiness") => Some(("connection", "check_readiness")),
        (&Method::GET, "/connections/{id}/binary") => Some(("connection", "download")),
        (&Method::POST, "/test/credentials") => Some(("credentials", "test")),
        (&Method::POST, "/flights") => Some(("flight_service", "create")),
        (&Method::PATCH, "/flights/{id}") => Some(("flight_service", "update")),
        (&Method::DELETE, "/flights/{id}") => Some(("flight_service", "delete")),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
/// log_request emits one structured audit event for a completed REST request.
pub fn log_request(
    request_id: &str,
    tenant_id: &str,
    actor: &str,
    resource: &str,
    action: &str,
    query: Option<&str>,
    resource_id: Option<&str>,
    status_code: u16,
    error: Option<&AuditError>,
    duration: Duration,
) {
    let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);

    match (resource_id, error) {
        (Some(resource_id), Some(error)) => tracing::info!(
            target: "audit",
            request_id,
            audit.tenant_id = tenant_id,
            audit.actor = actor,
            audit.resource = resource,
            audit.action = action,
            audit.query = query,
            resource_id,
            status_code,
            error_code = %error.code,
            error_message = %error.message,
            duration_ms
        ),
        (Some(resource_id), None) => tracing::info!(
            target: "audit",
            request_id,
            audit.tenant_id = tenant_id,
            audit.actor = actor,
            audit.resource = resource,
            audit.action = action,
            audit.query = query,
            resource_id,
            status_code,
            duration_ms
        ),
        (None, Some(error)) => tracing::info!(
            target: "audit",
            request_id,
            audit.tenant_id = tenant_id,
            audit.actor = actor,
            audit.resource = resource,
            audit.action = action,
            audit.query = query,
            status_code,
            error_code = %error.code,
            error_message = %error.message,
            duration_ms
        ),
        (None, None) => tracing::info!(
            target: "audit",
            request_id,
            audit.tenant_id = tenant_id,
            audit.actor = actor,
            audit.resource = resource,
            audit.action = action,
            audit.query = query,
            status_code,
            duration_ms
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_action_mapping_matches_public_api() {
        let cases = [
            (
                Method::GET,
                "/api/v1alpha1/data/connection-types",
                ("connection_type", "list"),
            ),
            (
                Method::POST,
                "/api/v1alpha1/data/connection-types",
                ("connection_type", "create"),
            ),
            (
                Method::GET,
                "/api/v1alpha1/data/connection-types/{id}",
                ("connection_type", "get"),
            ),
            (
                Method::PATCH,
                "/api/v1alpha1/data/connection-types/{id}",
                ("connection_type", "update"),
            ),
            (
                Method::DELETE,
                "/api/v1alpha1/data/connection-types/{id}",
                ("connection_type", "delete"),
            ),
            (Method::GET, "/api/v1alpha1/data/connections", ("connection", "list")),
            (Method::POST, "/api/v1alpha1/data/connections", ("connection", "create")),
            (
                Method::GET,
                "/api/v1alpha1/data/connections/{id}",
                ("connection", "get"),
            ),
            (
                Method::PATCH,
                "/api/v1alpha1/data/connections/{id}",
                ("connection", "update"),
            ),
            (
                Method::DELETE,
                "/api/v1alpha1/data/connections/{id}",
                ("connection", "delete"),
            ),
            (
                Method::PUT,
                "/api/v1alpha1/data/connections/{id}/exports/secrets/{secret_name}",
                ("connection_secret", "export"),
            ),
            (
                Method::POST,
                "/api/v1alpha1/data/connections/{id}/readiness",
                ("connection", "check_readiness"),
            ),
            (
                Method::GET,
                "/api/v1alpha1/data/connections/{id}/binary",
                ("connection", "download"),
            ),
            (
                Method::POST,
                "/api/v1alpha1/data/test/credentials",
                ("credentials", "test"),
            ),
            (Method::POST, "/api/v1alpha1/data/flights", ("flight_service", "create")),
            (
                Method::PATCH,
                "/api/v1alpha1/data/flights/{id}",
                ("flight_service", "update"),
            ),
            (
                Method::DELETE,
                "/api/v1alpha1/data/flights/{id}",
                ("flight_service", "delete"),
            ),
        ];

        for (method, route, expected) in cases {
            assert_eq!(resource_action_for(&method, route), Some(expected));
        }
    }

    #[test]
    fn resource_action_mapping_excludes_non_audited_routes() {
        assert_eq!(resource_action_for(&Method::GET, "/health"), None);
        assert_eq!(resource_action_for(&Method::GET, "/api/v1alpha1/data/missing"), None);
    }

    #[test]
    fn audit_config_is_disabled_by_default_and_requires_true() {
        assert!(!AuditConfig::from_value(None).enabled);
        assert!(!AuditConfig::from_value(Some("false")).enabled);
        assert!(!AuditConfig::from_value(Some("1")).enabled);
        assert!(AuditConfig::from_value(Some("TRUE")).enabled);
    }
}
