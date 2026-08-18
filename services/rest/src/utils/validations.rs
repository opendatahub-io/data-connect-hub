use commons::api::connections::Admin;
use commons::api::connections::DataConnection;

use commons::api::connections::Secret;

use std::collections::HashMap;
use std::sync::Arc;

pub async fn transform_data_connection(
    tenant_id: &str,
    data_connection: &DataConnection,
) -> (DataConnection, Option<Secret>) {
    let mut data_connection = data_connection.clone();

    match &data_connection.admin {
        Some(Admin::Secret { name, secret }) => {
            let properties = secret.clone();

            let secret_obj = Secret {
                name: name.to_string(),
                namespace: tenant_id.to_string(),
                properties: properties.clone(),
                labels: Arc::new(HashMap::new()),
                annotations: Arc::new(HashMap::new()),
            };
            data_connection.admin = Some(Admin::SecretRef {
                secret_ref: name.to_string(),
            });
            (data_connection, Some(secret_obj))
        },
        _ => (data_connection, None),
    }
}
