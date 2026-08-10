use tonic::service::Interceptor;
use tonic::{Request, Status};
use tracing::info;

#[derive(Debug, Default, Clone)]
pub struct AuthInterceptor;

impl AuthInterceptor {
    pub fn new() -> Self {
        Self
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        info!("AuthInterceptor: call");
        // TODO: Implement authentication and authorization
        // 0. Check cached info.
        // 1. Get the token from the request Authorization header.
        // 2. Perform TokenReview.
        // 3. Perform data-read SAR check for 'data-store' resource in the x-tenant-id namespace.
        // 4. Cache results with a TTL (moka::future::Cache).
        // 4. If invalid, return an error
        // 5. Return the request. Inject x-remote-user and x-remote-groups headers in the downstream requests. (similar with kube RBAC proxy behavior)
        Ok(request)
    }
}
