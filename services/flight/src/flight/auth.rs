use commons::api::{X_REMOTE_GROUPS, X_REMOTE_USER, X_TENANT_ID};
use http::header::AUTHORIZATION;
use http_body::Body;
use kube_utils::auth::{AuthError, KubeAuthClient};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tonic::Status;
use tower::{Layer, Service};
use tracing::{debug, warn};

const HEALTH_PATH_PREFIX: &str = "/grpc.health.v1.Health/";
const BEARER_PREFIX: &str = "Bearer ";

#[derive(Clone)]
pub struct AuthLayer {
    auth_service: Arc<KubeAuthClient>,
}

impl AuthLayer {
    pub fn new(auth_service: Arc<KubeAuthClient>) -> Self {
        Self { auth_service }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware {
            inner,
            auth_service: self.auth_service.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AuthMiddleware<S> {
    inner: S,
    auth_service: Arc<KubeAuthClient>,
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

        Box::pin(async move {
            let path = req.uri().path();
            if path.starts_with(HEALTH_PATH_PREFIX) {
                return inner.call(req).await;
            }

            let token = match extract_bearer_token(&req) {
                Some(t) => t.to_string(),
                None => {
                    warn!("Invalid or missing Authorization header");
                    return Ok(grpc_error_response(Status::unauthenticated(
                        "invalid or missing bearer token",
                    )));
                },
            };

            let tenant_id = match req
                .headers()
                .get(X_TENANT_ID)
                .and_then(|v| v.to_str().ok())
                .filter(|value| !value.is_empty())
            {
                Some(value) => value.to_string(),
                None => {
                    return Ok(grpc_error_response(Status::permission_denied(
                        "x-tenant-id header is required",
                    )));
                },
            };

            let auth_info = match auth_service.authenticate(&token).await {
                Ok(info) => info,
                Err(e) => {
                    return Ok(grpc_error_response(auth_error_to_status(&e)));
                },
            };

            if let Err(e) = auth_service.authorize(&auth_info, &tenant_id, "get").await {
                return Ok(grpc_error_response(auth_error_to_status(&e)));
            }

            debug!("Authenticated user={} for path={}", auth_info.username, path);

            let mut req = req;
            let headers = req.headers_mut();
            if let Ok(val) = http::HeaderValue::from_str(&auth_info.username) {
                headers.insert(X_REMOTE_USER, val);
            }
            let groups_joined = auth_info.groups.join(",");
            if let Ok(val) = http::HeaderValue::from_str(&groups_joined) {
                headers.insert(X_REMOTE_GROUPS, val);
            }

            inner.call(req).await
        })
    }
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
