use crate::flight::QueryContext;
use crate::flight::errors::map_connector_error;
use crate::flight::service::TabularDataService;
use arrow::array::{Array, StringArray};
use arrow::record_batch::RecordBatch;
use arrow_flight::{Action, ActionType, flight_service_server::FlightService};
use commons::api::ResourceMetadata;
use commons::api::connections::Admin;
use commons::api::connections::DataConnection;
use commons::api::connections::DataConnectionResource;
use commons::api::connections::DataConnectionStatus;
use commons::api::connections::DataFormat;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::info;
use uuid::Uuid;
const ACTION_GET_SUPPORTED_CONNECTORS: &str = "GetSupportedConnectors";
const ACTION_CHECK_CONNECTION: &str = "CheckConnection";

impl TabularDataService {
    pub fn custom_actions() -> Vec<Result<ActionType, Status>> {
        vec![
            Ok(ActionType {
                r#type: ACTION_GET_SUPPORTED_CONNECTORS.into(),
                description: "Returns the list of supported data connectors".into(),
            }),
            Ok(ActionType {
                r#type: ACTION_CHECK_CONNECTION.into(),
                description: "Checks the connection to the data source".into(),
            }),
        ]
    }

    pub async fn dispatch_action(
        &self,
        request: Request<Action>,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let action = request.get_ref();
        match action.r#type.as_str() {
            ACTION_CHECK_CONNECTION => self.action_check_connection(&request).await,
            ACTION_GET_SUPPORTED_CONNECTORS => self.action_get_supported_connectors().await,
            _ => Err(Status::invalid_argument(format!("Unknown action: {}", action.r#type))),
        }
    }

    async fn action_check_connection(
        &self,
        request: &Request<Action>,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let metadata = request.metadata();

        let tenant_id = QueryContext::tenant_id(metadata)?;

        let reader = if let Some(mut keys) = parse_credentials_body(&request.get_ref().body)? {
            let dct_id = keys
                .remove("data_connection_type_id")
                .ok_or(Status::invalid_argument("data_connection_type_id is required"))?;

            let credentials: Arc<HashMap<String, String>> = Arc::new(
                keys.into_iter()
                    .filter(|(k, _)| k.starts_with("secret."))
                    .map(|(k, v)| (k.strip_prefix("secret.").unwrap().to_string(), v))
                    .collect(),
            );

            let admin = Admin::Secret {
                name: String::new(),
                secret: credentials.clone(),
            };

            let (data_connection_type, connector) = self.get_connector_by_type_id(tenant_id, &dct_id).await?;

            data_connection_type
                .resource
                .check_credentials_schema(&credentials.clone())
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

            // Create a fake DataConnectionResource as this is not stored anywhere. We only need to pass the credentials to the connector.
            connector
                .get_reader(
                    false,
                    &DataConnectionResource {
                        metadata: ResourceMetadata {
                            id: Uuid::new_v4().to_string(),
                            tenant_id: Some(tenant_id.to_string()),
                            created_at: "2021-01-01".to_string(),
                            updated_at: "2021-01-01".to_string(),
                        },
                        resource: DataConnection {
                            name: "test".to_string(),
                            data_connection_type_id: dct_id.clone(),
                            format: DataFormat::Tabular,
                            admin: Some(admin),
                            properties: HashMap::new(),
                        },
                        status: DataConnectionStatus::default(),
                    },
                )
                .await
                .map_err(map_connector_error)?
        } else {
            let connection_id = QueryContext::connection_id(metadata)?;
            let (connection, connector) = self.get_connector_by_connection_id(tenant_id, connection_id).await?;
            connector
                .get_reader(true, &connection)
                .await
                .map_err(map_connector_error)?
        };

        reader.check_connection().await.map_err(map_connector_error)?;
        info!("Connection checked successfully");
        let result = arrow_flight::Result {
            body: Vec::new().into(),
        };
        Ok(Response::new(
            Box::pin(futures::stream::once(async { Ok(result) })) as <Self as FlightService>::DoActionStream
        ))
    }

    async fn action_get_supported_connectors(
        &self,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let connectors = self.connectors_registry.get_supported_connectors();
        let names: Vec<String> = connectors.iter().map(|c| c.provider()).collect();
        let descriptions: Vec<String> = connectors.iter().map(|c| c.description()).collect();

        let batch = RecordBatch::try_from_iter(vec![
            ("name", Arc::new(StringArray::from(names)) as _),
            ("description", Arc::new(StringArray::from(descriptions)) as _),
        ])
        .map_err(|e| {
            tracing::error!(error = %e, "failed to build connector record batch");
            Status::internal("failed to build response")
        })?;

        let mut buf = Vec::new();
        {
            let mut writer = arrow::ipc::writer::StreamWriter::try_new(&mut buf, &batch.schema()).map_err(|e| {
                tracing::error!(error = %e, "failed to create IPC writer");
                Status::internal("failed to encode response")
            })?;
            writer.write(&batch).map_err(|e| {
                tracing::error!(error = %e, "failed to write IPC batch");
                Status::internal("failed to encode response")
            })?;
            writer.finish().map_err(|e| {
                tracing::error!(error = %e, "failed to finish IPC stream");
                Status::internal("failed to encode response")
            })?;
        }

        let result = arrow_flight::Result { body: buf.into() };
        Ok(Response::new(
            Box::pin(futures::stream::once(async { Ok(result) })) as <Self as FlightService>::DoActionStream
        ))
    }
}

/// Parses an Arrow IPC stream containing credentials from the action body.
///
/// Returns `None` if the body is empty. Otherwise expects a single `RecordBatch`
/// with two `Utf8` columns:
///
/// | key (Utf8)                  | value (Utf8)  |
/// |-----------------------------|---------------|
/// | `data_connection_type_id`   | `<type-id>`   |
/// | `secret.<credential-name>`  | `<secret>`    |
///
/// The `data_connection_type_id` row is required. Rows prefixed with `secret.`
/// are collected (with the prefix stripped) into the credentials map.
fn parse_credentials_body(body: &[u8]) -> Result<Option<HashMap<String, String>>, Status> {
    if body.is_empty() {
        return Ok(None);
    }

    let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(body), None)
        .map_err(|e| Status::invalid_argument(format!("invalid credentials payload: {e}")))?;

    let batch: RecordBatch = reader
        .into_iter()
        .next()
        .ok_or_else(|| Status::invalid_argument("credentials payload is empty"))?
        .map_err(|e| Status::invalid_argument(format!("failed to read credentials: {e}")))?;

    let keys = batch
        .column_by_name("key")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| Status::invalid_argument("credentials payload missing 'key' column"))?;

    let values = batch
        .column_by_name("value")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| Status::invalid_argument("credentials payload missing 'value' column"))?;

    let map: HashMap<String, String> = (0..keys.len())
        .map(|i| (keys.value(i).to_string(), values.value(i).to_string()))
        .collect();

    Ok(Some(map))
}
