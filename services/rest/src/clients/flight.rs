use arrow::record_batch::RecordBatch;
use arrow_flight::Action;
use arrow_flight::flight_service_client::FlightServiceClient;
use tonic::transport::Channel;
use crate::utils::FlightService;

pub async fn get_supported_connectors(flight_service: &FlightService) -> Result<RecordBatch, tonic::Status> {
    let channel = Channel::from_shared(flight_service.endpoint())
        .map_err(|e| tonic::Status::internal(format!("invalid flight endpoint: {e}")))?
        .connect()
        .await
        .map_err(|e| tonic::Status::unavailable(format!("failed to connect to flight service: {e}")))?;

    let mut client = FlightServiceClient::new(channel);
    let action = Action::new("GetSupportedConnectors", "");

    let mut stream = client.do_action(action).await?.into_inner();
    let result = stream
        .message()
        .await?
        .ok_or_else(|| tonic::Status::internal("empty response from GetSupportedConnectors"))?;

    let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(result.body), None)
        .map_err(|e| tonic::Status::internal(format!("failed to read IPC stream: {e}")))?;

    let batches: Result<Vec<_>, _> = reader.collect();
    let batches = batches.map_err(|e| tonic::Status::internal(format!("failed to read IPC batches: {e}")))?;

    arrow::compute::concat_batches(&batches[0].schema(), &batches)
        .map_err(|e| tonic::Status::internal(format!("failed to concat batches: {e}")))
}