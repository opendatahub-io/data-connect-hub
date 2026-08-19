use commons::api::connections::Admin;

use commons::api::connections::DataConnectionResource;
use commons::api::storage::MetaStore;

use crate::rest::errors::ValidationError;
use commons::api::connections::DataFormat;
use commons::api::storage::SecretStore;
use std::sync::Arc;

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

        for field in dct.resource.credentials_fields.iter() {
            if field.required && !secret.properties.contains_key(field.name.as_str()) {
                return Err(ValidationError::MissingRequiredKey(field.name.clone()));
            }
        }
    } else {
        return Err(ValidationError::MissingRequiredKey("admin.secret_ref".to_string()));
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
