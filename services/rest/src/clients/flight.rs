use arrow::array::AsArray;
use arrow_flight::Action;
use arrow_flight::flight_service_client::FlightServiceClient;
use tokio::sync::OnceCell;
use tonic::transport::Channel;

#[derive(Debug, Clone)]
pub struct SupportedConnector {
    pub name: String,
    #[allow(dead_code)]
    pub description: String,
}

pub struct FlightClient {
    endpoint: String,
    client: OnceCell<FlightServiceClient<Channel>>,
}

impl FlightClient {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            client: OnceCell::new(),
        }
    }

    async fn client(&self) -> Result<FlightServiceClient<Channel>, tonic::Status> {
        self.client
            .get_or_try_init(|| async {
                let channel = Channel::from_shared(self.endpoint.clone())
                    .map_err(|e| tonic::Status::internal(format!("invalid flight endpoint: {e}")))?
                    .connect()
                    .await
                    .map_err(|e| tonic::Status::unavailable(format!("failed to connect to flight service: {e}")))?;
                Ok(FlightServiceClient::new(channel))
            })
            .await
            .cloned()
    }

    pub async fn get_supported_connectors(&self) -> Result<Vec<SupportedConnector>, tonic::Status> {
        let mut client = self.client().await?;
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

        if batches.is_empty() {
            return Err(tonic::Status::internal(
                "no batches returned from GetSupportedConnectors",
            ));
        }

        let batch = arrow::compute::concat_batches(&batches[0].schema(), &batches)
            .map_err(|e| tonic::Status::internal(format!("failed to concat batches: {e}")))?;

        let names = batch
            .column_by_name("name")
            .ok_or_else(|| tonic::Status::internal("missing 'name' column"))?
            .as_string::<i32>();

        let descriptions = batch
            .column_by_name("description")
            .ok_or_else(|| tonic::Status::internal("missing 'description' column"))?
            .as_string::<i32>();

        Ok((0..batch.num_rows())
            .map(|i| SupportedConnector {
                name: names.value(i).to_string(),
                description: descriptions.value(i).to_string(),
            })
            .collect())
    }
}
