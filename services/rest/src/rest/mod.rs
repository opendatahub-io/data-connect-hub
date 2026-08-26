pub mod endpoints;
pub mod errors;
pub mod middleware;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::clients::flight::FlightClient;
use commons::api::connection_types::DataConnectionTypeResource;
use commons::api::connection_types::DataConnectionTypeStatus;
use commons::api::storage::MetaStore;
use errors::RestErrorResponse;

pub(crate) async fn update_connection_type_status(
    flight_client: &FlightClient,
    meta_store: &Arc<dyn MetaStore + Send + Sync>,
    connection_type: DataConnectionTypeResource,
) -> Result<(), RestErrorResponse> {
    let connectors = flight_client.get_supported_connectors().await;

    if let Ok(connectors) = connectors {
        let names: Vec<String> = connectors.into_iter().map(|c| c.name).collect();
        let provider = &connection_type.resource.provider;

        let supports_flight = Arc::new(AtomicBool::new(names.iter().any(|n| n == provider)));

        let update_fn = Arc::new(move |current: DataConnectionTypeStatus| {
            let mut status = current.capabilities.clone();
            status.flight = supports_flight.load(Ordering::Relaxed);

            Ok(DataConnectionTypeStatus { capabilities: status })
        });

        meta_store
            .update_data_connection_type_status(connection_type.metadata.id.as_str(), update_fn)
            .await?;
    }
    Ok(())
}
