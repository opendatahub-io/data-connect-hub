use arrow::array::AsArray;
use arrow::array::StringArray;
use arrow::record_batch::RecordBatch;
use arrow_flight::Action;
use arrow_flight::FlightDescriptor;
use arrow_flight::decode::FlightRecordBatchStream;
use arrow_flight::error::FlightError;
use arrow_flight::flight_service_client::FlightServiceClient;
use commons::api::creds::TestCredentials;
use commons::api::{X_DATA_CONNECTION_ID, X_TENANT_ID};
use futures::TryStreamExt;
use prost::Message;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

const ACTION_CHECK_DATA_CONNECTION: &str = "CheckDataConnection";
const ACTION_CHECK_CREDENTIALS: &str = "CheckCredentials";
const DOWNLOAD_TYPE_URL: &str = "dataconnethub.opendatahub.io/download";

#[derive(Debug, Clone)]
pub struct SupportedConnector {
    pub name: String,
    #[allow(dead_code)]
    pub description: String,
}

pub type BinaryStream = Pin<Box<dyn futures::Stream<Item = Result<RecordBatch, tonic::Status>> + Send>>;

#[async_trait::async_trait]
pub trait FlightDataClient: Send + Sync {
    async fn get_supported_connectors(&self, token: Option<&str>) -> Result<Vec<SupportedConnector>, tonic::Status>;
    async fn check_data_connection(
        &self,
        tenant_id: &str,
        connection_id: &str,
        token: Option<&str>,
    ) -> Result<(), tonic::Status>;
    async fn test_credentials(
        &self,
        tenant_id: &str,
        creds: &TestCredentials,
        token: Option<&str>,
    ) -> Result<(), tonic::Status>;
    async fn download_binary(
        &self,
        tenant_id: &str,
        connection_id: &str,
        path: &str,
        token: Option<&str>,
    ) -> Result<BinaryStream, tonic::Status>;
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

    fn set_auth_token(metadata: &mut tonic::metadata::MetadataMap, token: Option<&str>) {
        if let Some(token) = token
            && let Ok(value) = MetadataValue::try_from(token)
        {
            metadata.insert("authorization", value);
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
}

#[async_trait::async_trait]
impl FlightDataClient for FlightClient {
    async fn get_supported_connectors(&self, token: Option<&str>) -> Result<Vec<SupportedConnector>, tonic::Status> {
        let mut client = self.client().await?;
        let mut request = tonic::Request::new(Action::new("GetSupportedConnectors", ""));
        Self::set_auth_token(request.metadata_mut(), token);

        let mut stream = client.do_action(request).await?.into_inner();
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

    async fn check_data_connection(
        &self,
        tenant_id: &str,
        connection_id: &str,
        token: Option<&str>,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client().await?;
        let mut request = tonic::Request::new(Action::new(ACTION_CHECK_DATA_CONNECTION, ""));
        let metadata = request.metadata_mut();
        Self::set_auth_token(metadata, token);
        metadata.insert(
            X_TENANT_ID,
            MetadataValue::try_from(tenant_id).map_err(|_| tonic::Status::invalid_argument("invalid tenant_id"))?,
        );
        metadata.insert(
            X_DATA_CONNECTION_ID,
            MetadataValue::try_from(connection_id)
                .map_err(|_| tonic::Status::invalid_argument("invalid connection_id"))?,
        );

        let mut stream = client.do_action(request).await?.into_inner();
        stream.message().await?;
        Ok(())
    }

    async fn test_credentials(
        &self,
        tenant_id: &str,
        creds: &TestCredentials,
        token: Option<&str>,
    ) -> Result<(), tonic::Status> {
        let mut keys = vec!["data_connection_type_id".to_string()];
        let mut values = vec![creds.data_connection_type_id.clone()];
        for (k, v) in &creds.secret {
            keys.push(format!("secret.{k}"));
            values.push(v.clone());
        }

        let batch = RecordBatch::try_from_iter(vec![
            ("key", Arc::new(StringArray::from(keys)) as _),
            ("value", Arc::new(StringArray::from(values)) as _),
        ])
        .map_err(|e| tonic::Status::internal(format!("failed to build credentials batch: {e}")))?;

        let mut buf = Vec::new();
        {
            let mut writer = arrow::ipc::writer::StreamWriter::try_new(&mut buf, &batch.schema())
                .map_err(|e| tonic::Status::internal(format!("failed to create IPC writer: {e}")))?;
            writer
                .write(&batch)
                .map_err(|e| tonic::Status::internal(format!("failed to write IPC batch: {e}")))?;
            writer
                .finish()
                .map_err(|e| tonic::Status::internal(format!("failed to finish IPC stream: {e}")))?;
        }

        let mut client = self.client().await?;
        let mut request = tonic::Request::new(Action::new(ACTION_CHECK_CREDENTIALS, buf));
        Self::set_auth_token(request.metadata_mut(), token);
        request.metadata_mut().insert(
            X_TENANT_ID,
            MetadataValue::try_from(tenant_id).map_err(|_| tonic::Status::invalid_argument("invalid tenant_id"))?,
        );

        let mut stream = client.do_action(request).await?.into_inner();
        stream.message().await?;
        Ok(())
    }

    async fn download_binary(
        &self,
        tenant_id: &str,
        connection_id: &str,
        path: &str,
        token: Option<&str>,
    ) -> Result<BinaryStream, tonic::Status> {
        let mut client = self.client().await?;

        let any = arrow_flight::sql::Any {
            type_url: DOWNLOAD_TYPE_URL.to_string(),
            value: path.as_bytes().to_vec().into(),
        };

        let descriptor = FlightDescriptor::new_cmd(any.encode_to_vec());

        let mut request = tonic::Request::new(descriptor);
        let metadata = request.metadata_mut();
        Self::set_auth_token(metadata, token);
        metadata.insert(
            X_TENANT_ID,
            MetadataValue::try_from(tenant_id).map_err(|_| tonic::Status::invalid_argument("invalid tenant_id"))?,
        );
        metadata.insert(
            X_DATA_CONNECTION_ID,
            MetadataValue::try_from(connection_id)
                .map_err(|_| tonic::Status::invalid_argument("invalid connection_id"))?,
        );

        let flight_info = client.get_flight_info(request).await?.into_inner();

        let ticket = flight_info
            .endpoint
            .into_iter()
            .next()
            .and_then(|e| e.ticket)
            .ok_or_else(|| tonic::Status::internal("no ticket in flight info response"))?;

        let mut request = tonic::Request::new(ticket);
        let metadata = request.metadata_mut();
        Self::set_auth_token(metadata, token);
        metadata.insert(
            X_TENANT_ID,
            MetadataValue::try_from(tenant_id).map_err(|_| tonic::Status::invalid_argument("invalid tenant_id"))?,
        );
        metadata.insert(
            X_DATA_CONNECTION_ID,
            MetadataValue::try_from(connection_id)
                .map_err(|_| tonic::Status::invalid_argument("invalid connection_id"))?,
        );

        let flight_stream = client.do_get(request).await?.into_inner();

        let batch_stream =
            FlightRecordBatchStream::new_from_flight_data(flight_stream.map_err(|e| FlightError::Tonic(Box::new(e))))
                .map_err(|e| match e {
                    FlightError::Tonic(status) => *status,
                    other => tonic::Status::internal(other.to_string()),
                });

        Ok(Box::pin(batch_stream))
    }
}
