use crate::flight::errors::map_connector_error;
use crate::flight::service::TabularDataService;
use arrow::array::StringArray;
use arrow::record_batch::RecordBatch;
use arrow_flight::{Action, ActionType, flight_service_server::FlightService};
use std::sync::Arc;

use crate::flight::QueryContext;
use tonic::{Request, Response, Status};

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
        let query_context = QueryContext::from_request(metadata)?;

        let (connection, connector) = self.get_connector(&query_context).await?;

        let reader = connector.get_reader(&connection).await.map_err(map_connector_error)?;

        reader.check_connection().await.map_err(map_connector_error)?;

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
