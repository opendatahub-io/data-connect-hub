use commons::api::connection_types::DataConnectionTypeStatus;
use commons::api::connections::Admin;

use commons::api::connections::DataConnectionResource;
use commons::api::storage::MetaStore;

use crate::clients::flight::FlightClient;
use crate::rest::errors::ValidationError;
use commons::api::connections::DataFormat;
use commons::api::storage::SecretStore;
use std::sync::Arc;
use tracing::info;

pub async fn verify_data_connection(
    data_connection: &DataConnectionResource,
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
) -> Result<(), ValidationError> {
    let tenant_id = data_connection
        .metadata
        .tenant_id
        .clone()
        .ok_or(ValidationError::InvalidTenantId)?;
    let dct = meta_store
        .get_data_connection_type(tenant_id.as_str(), &data_connection.resource.data_connection_type_id)
        .await
        .map_err(|_| ValidationError::InvalidDataConnectionType)?;

    if let Some(Admin::SecretRef { secret_ref }) = &data_connection.resource.admin {
        let secret = secret_store
            .get_secret(&tenant_id, secret_ref)
            .await
            .map_err(|_| ValidationError::InvalidSecret)?;

        dct.resource
            .check_credentials_schema(&secret.properties)
            .map_err(|e| ValidationError::CredentialsCheckFailed(e.to_string()))?;
    } else {
        return Err(ValidationError::MissingField("admin.secret_ref".to_string()));
    }

    match data_connection.resource.format {
        DataFormat::Tabular => {
            // TODO: Validate connection via Flight connectors
            // Update the data connection status if the connection is possible or not
        },
        DataFormat::Binary => {
            // TODO: Validate connection via Rest connectors
            // Update the data connection status if the connection is possible or not
        },
    }

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
