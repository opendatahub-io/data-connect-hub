use commons::api::connection_types::DataConnectionTypeStatus;
use commons::api::connections::Admin;

use commons::api::connections::DataConnectionResource;
use commons::api::storage::MetaStore;

use crate::clients::flight::FlightClient;
use crate::rest::errors::ValidationError;
use chrono::Utc;
use commons::api::connections::DataConnectionState;
use commons::api::connections::DataConnectionStatus;
use commons::api::connections::DataFormat;
use commons::api::storage::SecretStore;
use std::sync::Arc;
use tracing::info;

pub async fn verify_data_connection(
    tenant_id: &str,
    data_connection_id: &str,
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    flight_client: &FlightClient,
) -> Result<(), ValidationError> {
    let data_connection = meta_store
        .get_data_connection(tenant_id, data_connection_id)
        .await
        .map_err(|e| ValidationError::ConnectionCheckFailed(data_connection_id.to_string()))?;

    let dct = meta_store
        .get_data_connection_type(&tenant_id, &data_connection.resource.data_connection_type_id)
        .await
        .map_err(|_| ValidationError::InvalidDataConnectionType)?;

    let keys = match data_connection.resource.admin {
        Some(Admin::SecretRef { secret_ref }) => {
            let secret = secret_store
                .get_secret(&tenant_id, secret_ref.as_str())
                .await
                .map_err(|_| ValidationError::InvalidSecret)?;
            secret.properties
        },
        Some(Admin::Secret { name: _, secret }) => secret,
        _ => {
            return Err(ValidationError::MissingField("admin.secret_ref".to_string()));
        },
    };

    dct.resource
        .check_credentials_schema(&keys)
        .map_err(|e| ValidationError::CredentialsCheckFailed(e.to_string()))?;

    let connection_id = data_connection.metadata.id.clone();
    let result = flight_client.check_data_connection(&tenant_id, &connection_id).await;

    match result {
        Ok(_) => {
            let update_fn = Arc::new(|_: DataConnectionStatus| {
                let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                Ok(DataConnectionStatus {
                    state: DataConnectionState::Ready,
                    message: Some("Connection check successful".to_string()),
                    updated_at: Some(now),
                    phases: vec![],
                })
            });
            meta_store
                .update_data_connection_status(&connection_id, update_fn)
                .await
                .map_err(|_| {
                    ValidationError::StatusUpdateFailed("Failed to update data connection status".to_string())
                })?;
        },
        Err(_) => {
            let update_fn = Arc::new(|_: DataConnectionStatus| {
                let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                Ok(DataConnectionStatus {
                    state: DataConnectionState::NotReady,
                    message: Some("Connection check failed".to_string()),
                    updated_at: Some(now),
                    phases: vec![],
                })
            });
            meta_store
                .update_data_connection_status(&connection_id, update_fn)
                .await
                .map_err(|_| {
                    ValidationError::StatusUpdateFailed("Failed to update data connection status".to_string())
                })?;

            return Err(ValidationError::ConnectionCheckFailed(connection_id).into());
        },
    };
    Ok(())
}

pub async fn audit_data_connection_types(
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    flight_client: &FlightClient,
) -> Result<(), ValidationError> {
    let supported = flight_client.get_supported_connectors().await.map_err(|e| {
        tracing::error!(error = %e, "failed to get supported connectors from flight service");
        ValidationError::FlightServiceError(e.to_string())
    })?;

    let supported_names: Vec<&str> = supported.iter().map(|c| c.name.as_str()).collect();

    info!("supported connectors: {:?}", supported_names.join(", "));

    let data_connection_types = meta_store
        .get_all_data_connection_types()
        .await
        .map_err(|_| ValidationError::InvalidDataConnectionType)?;

    for dct in &data_connection_types.items {
        info!(
            "Checking data connection type: {} {:?}",
            dct.resource.name, dct.resource.provider
        );

        let mut capabilities = dct.status.capabilities.clone();
        let flight = supported_names.contains(&dct.resource.provider.as_str());
        if capabilities.flight != flight {
            capabilities.flight = flight;

            info!("Capabilities after update: {:?}", capabilities);

            let update_fn = Arc::new(move |current: DataConnectionTypeStatus| {
                let mut status = current.capabilities.clone();
                status.flight = capabilities.flight;
                Ok(DataConnectionTypeStatus { capabilities: status })
            });
            meta_store
                .update_data_connection_type_status(&dct.metadata.id, update_fn)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, provider = %dct.resource.provider, "failed to update connection type status");
                    ValidationError::InvalidDataConnectionType
                })?;
            info!("updated data connection type status: {:?}", dct.status);
        }
    }

    Ok(())
}
