use commons::api::{X_DATA_CONNECTION_ID, X_TENANT_ID};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tonic::{Code, Request, Status};

/// AuditConfig controls whether Flight audit records are emitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuditConfig {
    /// enabled indicates whether Flight audit events should be emitted.
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

#[derive(Clone, Debug)]
pub(crate) struct RpcRequestId(pub String);

#[derive(Clone, Debug)]
pub(crate) struct CallerIdentity {
    pub actor: String,
    pub caller_principal: String,
    pub parent_request_id: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct RpcAuditState {
    logged: AtomicBool,
    identity: Mutex<Option<CallerIdentity>>,
}

impl RpcAuditState {
    pub fn set_identity(&self, identity: CallerIdentity) {
        *self.identity.lock().unwrap() = Some(identity);
    }

    fn identity(&self) -> Option<CallerIdentity> {
        self.identity.lock().unwrap().clone()
    }

    fn claim(&self) -> bool {
        !self.logged.swap(true, Ordering::AcqRel)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AuditContext {
    pub request_id: String,
    pub parent_request_id: Option<String>,
    pub actor: String,
    pub caller_principal: String,
    pub tenant_id: String,
    pub resource_id: Option<String>,
    /// query records what the request actually read: the SQL statement for
    /// query operations or the file path for binary downloads.
    pub query: Option<String>,
    pub(crate) state: Option<Arc<RpcAuditState>>,
}

impl AuditContext {
    pub fn from_request<T>(request: &Request<T>) -> Self {
        let state = request.extensions().get::<Arc<RpcAuditState>>().cloned();
        let identity = request
            .extensions()
            .get::<CallerIdentity>()
            .cloned()
            .or_else(|| state.as_ref().and_then(|state| state.identity()));
        Self {
            request_id: request
                .extensions()
                .get::<RpcRequestId>()
                .map(|id| id.0.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            parent_request_id: identity
                .as_ref()
                .and_then(|identity| identity.parent_request_id.clone()),
            actor: identity
                .as_ref()
                .map(|identity| identity.actor.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            caller_principal: identity
                .as_ref()
                .map(|identity| identity.caller_principal.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            tenant_id: metadata_value(request, X_TENANT_ID).unwrap_or_default(),
            resource_id: metadata_value(request, X_DATA_CONNECTION_ID),
            query: None,
            state,
        }
    }

    pub(crate) fn from_http(
        request_id: String,
        tenant_id: String,
        resource_id: Option<String>,
        state: Arc<RpcAuditState>,
    ) -> Self {
        let identity = state.identity();
        Self {
            request_id,
            parent_request_id: identity
                .as_ref()
                .and_then(|identity| identity.parent_request_id.clone()),
            actor: identity
                .as_ref()
                .map(|identity| identity.actor.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            caller_principal: identity
                .as_ref()
                .map(|identity| identity.caller_principal.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            tenant_id,
            resource_id,
            query: None,
            state: Some(state),
        }
    }
}

fn metadata_value<T>(request: &Request<T>, key: &'static str) -> Option<String> {
    request
        .metadata()
        .get(key)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn grpc_status_label(code: Code) -> &'static str {
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "Cancelled",
        Code::Unknown => "Unknown",
        Code::InvalidArgument => "InvalidArgument",
        Code::DeadlineExceeded => "DeadlineExceeded",
        Code::NotFound => "NotFound",
        Code::AlreadyExists => "AlreadyExists",
        Code::PermissionDenied => "PermissionDenied",
        Code::ResourceExhausted => "ResourceExhausted",
        Code::FailedPrecondition => "FailedPrecondition",
        Code::Aborted => "Aborted",
        Code::OutOfRange => "OutOfRange",
        Code::Unimplemented => "Unimplemented",
        Code::Internal => "Internal",
        Code::Unavailable => "Unavailable",
        Code::DataLoss => "DataLoss",
        Code::Unauthenticated => "Unauthenticated",
    }
}

/// MAX_AUDIT_QUERY_LEN bounds the `audit.query` field so an unusually large
/// SQL statement cannot flood the shared log stream.
const MAX_AUDIT_QUERY_LEN: usize = 4096;

/// bounded_query returns `query` unchanged when it fits MAX_AUDIT_QUERY_LEN
/// and a truncated copy marked with an ellipsis otherwise.
pub(crate) fn bounded_query(query: String) -> String {
    if query.len() <= MAX_AUDIT_QUERY_LEN {
        return query;
    }
    let mut end = MAX_AUDIT_QUERY_LEN;
    while end > 0 && !query.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = query[..end].to_owned();
    truncated.push_str("...[truncated]");
    truncated
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn log_rpc(
    enabled: bool,
    context: &AuditContext,
    resource: &'static str,
    action: &'static str,
    status: Option<&Status>,
    duration: Duration,
) {
    if !enabled {
        return;
    }
    if context.state.as_ref().is_some_and(|state| !state.claim()) {
        return;
    }
    let code = status.map_or(Code::Ok, Status::code);
    let grpc_status = grpc_status_label(code);
    let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    let current_identity = context.state.as_ref().and_then(|state| state.identity());
    let actor = current_identity
        .as_ref()
        .map(|identity| identity.actor.as_str())
        .unwrap_or(&context.actor);
    let caller_principal = current_identity
        .as_ref()
        .map(|identity| identity.caller_principal.as_str())
        .unwrap_or(&context.caller_principal);
    let parent_request_id = current_identity
        .as_ref()
        .and_then(|identity| identity.parent_request_id.as_deref())
        .or(context.parent_request_id.as_deref());
    let resource_id = context.resource_id.as_deref();

    match (parent_request_id, resource_id, status) {
        (Some(parent_request_id), Some(resource_id), Some(_)) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            parent_request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            resource_id,
            grpc_status,
            error_code = grpc_status,
            error_message = "gRPC request failed",
            duration_ms
        ),
        (Some(parent_request_id), Some(resource_id), None) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            parent_request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            resource_id,
            grpc_status,
            duration_ms
        ),
        (Some(parent_request_id), None, Some(_)) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            parent_request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            grpc_status,
            error_code = grpc_status,
            error_message = "gRPC request failed",
            duration_ms
        ),
        (Some(parent_request_id), None, None) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            parent_request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            grpc_status,
            duration_ms
        ),
        (None, Some(resource_id), Some(_)) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            resource_id,
            grpc_status,
            error_code = grpc_status,
            error_message = "gRPC request failed",
            duration_ms
        ),
        (None, Some(resource_id), None) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            resource_id,
            grpc_status,
            duration_ms
        ),
        (None, None, Some(_)) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            grpc_status,
            error_code = grpc_status,
            error_message = "gRPC request failed",
            duration_ms
        ),
        (None, None, None) => tracing::info!(
            target: "audit",
            request_id = %context.request_id,
            audit.actor = actor,
            caller_principal,
            audit.tenant_id = %context.tenant_id,
            audit.resource = resource,
            audit.action = action,
            audit.query = context.query.as_deref(),
            grpc_status,
            duration_ms
        ),
    }
}

pub(crate) struct PendingAudit {
    enabled: bool,
    context: AuditContext,
    resource: &'static str,
    action: &'static str,
    started: std::time::Instant,
    finished: bool,
}

impl PendingAudit {
    pub fn new(
        enabled: bool,
        context: AuditContext,
        resource: &'static str,
        action: &'static str,
        started: std::time::Instant,
    ) -> Self {
        Self {
            enabled,
            context,
            resource,
            action,
            started,
            finished: false,
        }
    }

    pub fn complete(&mut self, status: Option<&Status>) {
        if self.finished {
            return;
        }
        self.finished = true;
        log_rpc(
            self.enabled,
            &self.context,
            self.resource,
            self.action,
            status,
            self.started.elapsed(),
        );
    }

    pub fn transfer_to_stream(&mut self) {
        self.finished = true;
    }
}

impl Drop for PendingAudit {
    fn drop(&mut self) {
        if !self.finished {
            let status = Status::cancelled("request cancelled");
            self.complete(Some(&status));
        }
    }
}

pub(crate) fn do_get_operation(operation: &str) -> (&'static str, &'static str) {
    match operation {
        "sql_info" => ("flight_server", "get_sql_info"),
        "tables" => ("connection", "list_tables"),
        "statement" => ("connection", "query"),
        "binary" => ("connection", "download_binary"),
        _ => ("flight_server", "do_get"),
    }
}

pub(crate) fn action_operation(action: &str) -> (&'static str, &'static str) {
    match action {
        "CheckDataConnection" => ("connection", "check_readiness"),
        "CheckCredentials" => ("credentials", "check"),
        "GetSupportedConnectors" => ("connector_catalog", "list"),
        _ => ("flight_server", "do_action"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn do_get_operations_match_documented_actions() {
        assert_eq!(do_get_operation("sql_info"), ("flight_server", "get_sql_info"));
        assert_eq!(do_get_operation("tables"), ("connection", "list_tables"));
        assert_eq!(do_get_operation("statement"), ("connection", "query"));
        assert_eq!(do_get_operation("binary"), ("connection", "download_binary"));
    }

    #[test]
    fn action_operations_match_documented_actions() {
        assert_eq!(
            action_operation("CheckDataConnection"),
            ("connection", "check_readiness")
        );
        assert_eq!(action_operation("CheckCredentials"), ("credentials", "check"));
        assert_eq!(
            action_operation("GetSupportedConnectors"),
            ("connector_catalog", "list")
        );
    }

    #[test]
    fn audit_config_is_disabled_by_default_and_requires_true() {
        assert!(!AuditConfig::from_value(None).enabled);
        assert!(!AuditConfig::from_value(Some("false")).enabled);
        assert!(!AuditConfig::from_value(Some("1")).enabled);
        assert!(AuditConfig::from_value(Some("TRUE")).enabled);
    }

    #[test]
    fn rpc_audit_state_can_only_be_claimed_once() {
        let state = RpcAuditState::default();

        assert!(state.claim());
        assert!(!state.claim());
    }

    #[test]
    fn bounded_query_keeps_short_queries_intact() {
        assert_eq!(bounded_query("SELECT 1".to_string()), "SELECT 1");
    }

    #[test]
    fn bounded_query_truncates_long_queries_on_char_boundary() {
        let query = format!("x{}", "é".repeat(MAX_AUDIT_QUERY_LEN));
        let bounded = bounded_query(query);

        assert!(bounded.ends_with("...[truncated]"));
        let kept = bounded.strip_suffix("...[truncated]").unwrap();
        assert!(kept.chars().count() < MAX_AUDIT_QUERY_LEN);
        assert!(kept.chars().last().is_some_and(|c| c == 'é'));
    }
}
