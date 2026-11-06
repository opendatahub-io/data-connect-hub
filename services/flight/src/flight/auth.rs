use crate::flight::audit::{AuditConfig, AuditContext, CallerIdentity, PendingAudit, RpcRequestId};
use commons::api::{X_REMOTE_USER, X_REQUEST_ID, X_TENANT_ID};
use http::header::AUTHORIZATION;
use http_body::Body;
use kube_utils::auth::{AuthError, KubeAuthClient};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tonic::Status;
use tower::{Layer, Service};
use tracing::{debug, warn};

const HEALTH_PATH_PREFIX: &str = "/grpc.health.v1.Health/";
const DO_ACTION_PATH: &str = "/arrow.flight.protocol.FlightService/DoAction";
const BEARER_PREFIX: &str = "Bearer ";

#[derive(Clone)]
pub struct AuthLayer {
    auth_service: Arc<KubeAuthClient>,
    discovery_service_account: String,
    audit: AuditConfig,
}

impl AuthLayer {
    pub fn new(auth_service: Arc<KubeAuthClient>, discovery_service_account: String, audit: AuditConfig) -> Self {
        Self {
            auth_service,
            discovery_service_account,
            audit,
        }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware {
            inner,
            auth_service: self.auth_service.clone(),
            discovery_service_account: self.discovery_service_account.clone(),
            audit: self.audit,
        }
    }
}

#[derive(Clone)]
pub struct AuthMiddleware<S> {
    inner: S,
    auth_service: Arc<KubeAuthClient>,
    discovery_service_account: String,
    audit: AuditConfig,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for AuthMiddleware<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Body + Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let auth_service = self.auth_service.clone();
        let discovery_service_account = self.discovery_service_account.clone();
        let audit = self.audit;

        Box::pin(async move {
            let started = Instant::now();
            let path = req.uri().path();
            if path.starts_with(HEALTH_PATH_PREFIX) {
                return inner.call(req).await;
            }
            let mut authentication_audit = pending_auth_audit(audit, &req, None, started);

            let token = match extract_bearer_token(&req) {
                Some(t) => t.to_string(),
                None => {
                    warn!("Invalid or missing Authorization header");
                    let status = Status::unauthenticated("invalid or missing bearer token");
                    authentication_audit.complete(Some(&status));
                    return Ok(grpc_error_response(status));
                },
            };

            let tenant_id = req
                .headers()
                .get(X_TENANT_ID)
                .and_then(|v| v.to_str().ok())
                .filter(|value| !value.is_empty())
                .map(str::to_string);

            let auth_info = match auth_service.authenticate(&token).await {
                Ok(info) => info,
                Err(e) => {
                    let status = auth_error_to_status(&e);
                    authentication_audit.complete(Some(&status));
                    return Ok(grpc_error_response(status));
                },
            };
            let identity = caller_identity(&req, &auth_info.username, &discovery_service_account);
            if let Some(state) = req.extensions().get::<Arc<crate::flight::audit::RpcAuditState>>() {
                state.set_identity(identity.clone());
            }
            authentication_audit.transfer_to_stream();
            let mut authorization_audit = pending_auth_audit(audit, &req, Some(&auth_info.username), started);

            if tenant_id.is_none() && (path != DO_ACTION_PATH || auth_info.username != discovery_service_account) {
                let status = Status::permission_denied("x-tenant-id header is required");
                authorization_audit.complete(Some(&status));
                return Ok(grpc_error_response(status));
            }

            if let Some(tenant_id) = tenant_id
                && let Err(e) = auth_service.authorize(&auth_info, &tenant_id, "get").await
            {
                let status = auth_error_to_status(&e);
                authorization_audit.complete(Some(&status));
                return Ok(grpc_error_response(status));
            }
            authorization_audit.transfer_to_stream();

            debug!("Authenticated user={} for path={}", auth_info.username, path);

            let mut req = req;
            req.extensions_mut().insert(identity);

            inner.call(req).await
        })
    }
}

fn caller_identity<B>(req: &http::Request<B>, principal: &str, trusted_principal: &str) -> CallerIdentity {
    let trusted = !trusted_principal.is_empty() && principal == trusted_principal;
    let actor = trusted
        .then(|| header_value(req, X_REMOTE_USER))
        .flatten()
        .unwrap_or_else(|| principal.to_string());
    let parent_request_id = trusted
        .then(|| header_value(req, X_REQUEST_ID))
        .flatten()
        .filter(|value| uuid::Uuid::parse_str(value).is_ok());
    CallerIdentity {
        actor,
        caller_principal: principal.to_string(),
        parent_request_id,
    }
}

fn header_value<B>(req: &http::Request<B>, key: &'static str) -> Option<String> {
    req.headers()
        .get(key)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn pending_auth_audit<B>(
    audit: AuditConfig,
    req: &http::Request<B>,
    principal: Option<&str>,
    started: Instant,
) -> PendingAudit {
    let principal = principal.unwrap_or("unknown");
    let context = AuditContext {
        request_id: req
            .extensions()
            .get::<RpcRequestId>()
            .map(|id| id.0.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        parent_request_id: None,
        actor: principal.to_string(),
        caller_principal: principal.to_string(),
        tenant_id: header_value(req, X_TENANT_ID).unwrap_or_default(),
        resource_id: None,
        query: None,
        state: req
            .extensions()
            .get::<Arc<crate::flight::audit::RpcAuditState>>()
            .cloned(),
    };
    PendingAudit::new(
        audit.enabled && is_audited_path(req.uri().path()),
        context,
        "flight_server",
        "authenticate",
        started,
    )
}

fn is_audited_path(path: &str) -> bool {
    matches!(path, DO_ACTION_PATH | "/arrow.flight.protocol.FlightService/DoGet")
}

fn extract_bearer_token<B>(req: &http::Request<B>) -> Option<&str> {
    req.headers()
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix(BEARER_PREFIX)
}

fn auth_error_to_status(e: &AuthError) -> Status {
    match e {
        AuthError::Unauthenticated(msg) => Status::unauthenticated(msg),
        AuthError::Forbidden(msg) => Status::permission_denied(msg),
        AuthError::TokenReviewError(_) => {
            warn!("{e}");
            Status::internal("token verification failed")
        },
        AuthError::AccessReviewError(_) => {
            warn!("{e}");
            Status::internal("authorization check failed")
        },
        AuthError::Internal(msg) => {
            warn!("{e}");
            Status::internal(msg)
        },
    }
}

fn grpc_error_response<ResBody: Default>(status: Status) -> http::Response<ResBody> {
    status.into_http()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRUSTED: &str = "system:serviceaccount:dch:rest";

    #[test]
    fn trusted_rest_principal_forwards_actor_and_parent_request() {
        let parent = uuid::Uuid::new_v4().to_string();
        let request = http::Request::builder()
            .header(X_REMOTE_USER, "alice")
            .header(X_REQUEST_ID, &parent)
            .body(())
            .unwrap();

        let identity = caller_identity(&request, TRUSTED, TRUSTED);

        assert_eq!(identity.actor, "alice");
        assert_eq!(identity.caller_principal, TRUSTED);
        assert_eq!(identity.parent_request_id.as_deref(), Some(parent.as_str()));
    }

    #[test]
    fn untrusted_principal_cannot_spoof_actor_or_parent_request() {
        let request = http::Request::builder()
            .header(X_REMOTE_USER, "alice")
            .header(X_REQUEST_ID, uuid::Uuid::new_v4().to_string())
            .body(())
            .unwrap();

        let identity = caller_identity(&request, "direct-user", TRUSTED);

        assert_eq!(identity.actor, "direct-user");
        assert_eq!(identity.caller_principal, "direct-user");
        assert_eq!(identity.parent_request_id, None);
    }

    #[test]
    fn trusted_principal_ignores_invalid_parent_request_id() {
        let request = http::Request::builder()
            .header(X_REQUEST_ID, "not-a-uuid")
            .body(())
            .unwrap();

        let identity = caller_identity(&request, TRUSTED, TRUSTED);

        assert_eq!(identity.actor, TRUSTED);
        assert_eq!(identity.parent_request_id, None);
    }

    #[test]
    fn auth_failure_auditing_excludes_discovery_rpcs() {
        assert!(is_audited_path(DO_ACTION_PATH));
        assert!(is_audited_path("/arrow.flight.protocol.FlightService/DoGet"));
        assert!(!is_audited_path("/arrow.flight.protocol.FlightService/GetFlightInfo"));
        assert!(!is_audited_path("/arrow.flight.protocol.FlightService/ListActions"));
        assert!(!is_audited_path("/grpc.health.v1.Health/Check"));
    }
}
