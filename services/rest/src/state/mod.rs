pub mod audit;
pub use audit::*;

use std::sync::Arc;

use crate::auth::RequestAuthorizer;
use crate::clients::flight::FlightClient;
use crate::clients::flight::FlightDataClient;
use commons::api::storage::MetaStore;
use commons::api::storage::SecretStore;

pub(crate) struct ApiService {
    pub meta_store: Arc<dyn MetaStore + Send + Sync>,
    pub secret_store: Arc<dyn SecretStore + Send + Sync>,
    pub flight_ca_cert: Option<Vec<u8>>,
    pub service_account_token_file: Option<String>,
    pub global_tenant_id: String,
    pub authorizer: Option<Arc<dyn RequestAuthorizer>>,
}

impl ApiService {
    pub fn new(
        meta_store: Arc<dyn MetaStore + Send + Sync>,
        secret_store: Arc<dyn SecretStore + Send + Sync>,
        flight_ca_cert: Option<Vec<u8>>,
        service_account_token_file: Option<String>,
        global_tenant_id: String,
    ) -> Self {
        Self {
            meta_store,
            secret_store,
            flight_ca_cert,
            service_account_token_file,
            global_tenant_id,
            authorizer: None,
        }
    }

    /// with_authorizer enables requester checks on secret references. When no
    /// authorizer is set the service resolves `credentials_ref` secrets
    /// without verifying the requester (development mode).
    pub fn with_authorizer(mut self, authorizer: Arc<dyn RequestAuthorizer>) -> Self {
        self.authorizer = Some(authorizer);
        self
    }

    pub fn flight_client(&self, endpoint: &str) -> Arc<dyn FlightDataClient + Send + Sync> {
        Arc::new(FlightClient::new(
            endpoint.to_string(),
            self.flight_ca_cert.clone(),
            self.service_account_token_file.clone(),
        ))
    }
}
