use std::sync::Arc;

use arrow::array::{Array, Float64Array, Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use arrow_flight::{flight_service_server::FlightServiceServer, sql::client::FlightSqlServiceClient};
use commons::api::connection_types::DataConnectionType;
use commons::api::connection_types::DataConnectionTypeResource;
use commons::api::connection_types::Secret;
use commons::api::connections::{Admin, DataConnection};
use commons::api::connections::{DataConnectionResource, DataFormat};
use commons::api::errors::MetaStoreError;
use commons::api::storage::MetaStore;
use commons::api::{ResourceList, ResourceMetadata, X_DATA_CONNECTION_ID, X_TENANT_ID};
use flight_service::flight::registry::ConnectorsRegistry;
use flight_service::flight::service::TabularDataService;

mod common;
use common::InMemorySecretStore;
use futures::TryStreamExt;
use opendal::{Operator, services::Memory};
use s3_connector::S3Connector;
use std::collections::HashMap;
use std::time::Duration;
use tokio::net::TcpListener;
use tonic::transport::{Channel, Server};

struct S3TestMetaStore {
    format: DataFormat,
}

#[async_trait::async_trait]
impl MetaStore for S3TestMetaStore {
    async fn get_data_connections(
        &self,
        _tenant_id: &str,
    ) -> Result<ResourceList<DataConnectionResource>, MetaStoreError> {
        unimplemented!()
    }

    async fn get_data_connection(
        &self,
        _tenant_id: &str,
        _connection_id: &str,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        Ok(DataConnectionResource {
            metadata: ResourceMetadata {
                id: "s3-conn-1".to_string(),
                tenant_id: Some("default".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: DataConnection {
                name: "test-s3".to_string(),
                data_connection_type_id: "s3-type".to_string(),
                format: self.format.clone(),
                admin: Some(Admin::SecretRef {
                    secret_ref: "s3_creds".to_string(),
                }),
                properties: HashMap::new(),
            },
            status: Default::default(),
        })
    }

    async fn create_data_connection(
        &self,
        _tenant_id: &str,
        _data_connection: &DataConnection,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        unimplemented!()
    }

    async fn update_data_connection(
        &self,
        _tenant_id: &str,
        _uid: &str,
        _update_fn: Arc<dyn Fn(DataConnection) -> Result<DataConnection, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        unimplemented!()
    }

    async fn delete_data_connection(&self, _tenant_id: &str, _uid: &str) -> Result<(), MetaStoreError> {
        unimplemented!()
    }

    async fn get_data_connection_types(
        &self,
        _tenant_id: &str,
    ) -> Result<ResourceList<DataConnectionTypeResource>, MetaStoreError> {
        unimplemented!()
    }

    async fn get_data_connection_type(
        &self,
        _tenant_id: &str,
        _id: &str,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        Ok(DataConnectionTypeResource {
            metadata: ResourceMetadata {
                id: "s3-type".to_string(),
                tenant_id: Some("default".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: DataConnectionType {
                name: "S3".to_string(),
                provider: "s3".to_string(),
                description: Some("S3-compatible object storage".to_string()),
                credentials_fields: vec![],
            },
            status: Default::default(),
        })
    }

    async fn create_data_connection_type(
        &self,
        _tenant_id: &str,
        _data_connection_type: &DataConnectionType,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        unimplemented!()
    }

    async fn update_data_connection_type(
        &self,
        _tenant_id: &str,
        _uid: &str,
        _update_fn: Arc<dyn Fn(DataConnectionType) -> Result<DataConnectionType, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        unimplemented!()
    }

    async fn delete_data_connection_type(&self, _tenant_id: &str, _uid: &str) -> Result<(), MetaStoreError> {
        unimplemented!()
    }
}

fn write_parquet_to_bytes(batch: &RecordBatch) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut writer = parquet::arrow::ArrowWriter::try_new(&mut buf, batch.schema(), None).unwrap();
    writer.write(batch).unwrap();
    writer.close().unwrap();
    buf
}

fn write_csv_to_bytes(batch: &RecordBatch) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut writer = arrow_csv::WriterBuilder::new().with_header(true).build(&mut buf);
    writer.write(batch).unwrap();
    drop(writer);
    buf
}

fn sample_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("score", DataType::Float64, true),
    ]));

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5])),
            Arc::new(StringArray::from(vec!["alice", "bob", "charlie", "diana", "eve"])),
            Arc::new(Float64Array::from(vec![
                Some(95.5),
                Some(87.0),
                Some(92.3),
                None,
                Some(88.1),
            ])),
        ],
    )
    .unwrap()
}

fn test_credentials() -> HashMap<String, String> {
    HashMap::from([
        ("AWS_S3_BUCKET".to_string(), "test-bucket".to_string()),
        ("AWS_ACCESS_KEY_ID".to_string(), "test-key".to_string()),
        ("AWS_SECRET_ACCESS_KEY".to_string(), "test-secret".to_string()),
    ])
}

async fn setup_memory_operator(path: &str, data: Vec<u8>) -> Operator {
    let op = Operator::new(Memory::default()).unwrap();
    op.write(path, data).await.unwrap();
    op
}

async fn start_flight_server(meta_store: impl MetaStore + Send + Sync + 'static, operator: Operator) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let connector = S3Connector::new(Duration::from_secs(300), Duration::from_secs(60), 10);
    connector.insert_operator("s3-conn-1", operator).await;

    let connectors_registry = ConnectorsRegistry::new().with_connector(Arc::new(connector));

    let secret_store = InMemorySecretStore::new(vec![Secret {
        name: "s3_creds".to_string(),
        namespace: "default".to_string(),
        properties: Arc::new(test_credentials()),
        labels: Arc::new(HashMap::new()),
        annotations: Arc::new(HashMap::new()),
    }]);

    let service = TabularDataService::new(
        Arc::new(connectors_registry),
        Arc::new(meta_store),
        Arc::new(secret_store),
        Default::default(),
    );

    tokio::spawn(async move {
        Server::builder()
            .add_service(FlightServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    format!("http://{addr}")
}

async fn flight_client(url: &str) -> FlightSqlServiceClient<Channel> {
    let channel = Channel::from_shared(url.to_string()).unwrap().connect().await.unwrap();

    let mut client = FlightSqlServiceClient::new(channel);
    client.set_header(X_DATA_CONNECTION_ID, "s3-conn-1");
    client.set_header(X_TENANT_ID, "default");
    client
}

#[tokio::test]
async fn test_flight_s3_read_parquet() {
    let batch = sample_batch();
    let parquet_bytes = write_parquet_to_bytes(&batch);
    let op = setup_memory_operator("data/test.parquet", parquet_bytes).await;

    let url = start_flight_server(
        S3TestMetaStore {
            format: DataFormat::Tabular,
        },
        op,
    )
    .await;

    let mut client = flight_client(&url).await;

    let flight_info = client.execute("data/test.parquet".to_string(), None).await.unwrap();

    let ticket = flight_info.endpoint[0].ticket.clone().unwrap();
    let stream = client.do_get(ticket).await.unwrap();
    let batches: Vec<_> = stream.try_collect().await.unwrap();

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 5);

    let names = batches[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(names.value(0), "alice");
    assert_eq!(names.value(4), "eve");

    let scores = batches[0].column(2).as_any().downcast_ref::<Float64Array>().unwrap();
    assert!((scores.value(0) - 95.5).abs() < f64::EPSILON);
    assert!(scores.is_null(3));
}

#[tokio::test]
async fn test_flight_s3_read_csv() {
    let batch = sample_batch();
    let csv_bytes = write_csv_to_bytes(&batch);
    let op = setup_memory_operator("data/test.csv", csv_bytes).await;

    let url = start_flight_server(
        S3TestMetaStore {
            format: DataFormat::Tabular,
        },
        op,
    )
    .await;

    let mut client = flight_client(&url).await;

    let flight_info = client.execute("data/test.csv".to_string(), None).await.unwrap();

    let ticket = flight_info.endpoint[0].ticket.clone().unwrap();
    let stream = client.do_get(ticket).await.unwrap();
    let batches: Vec<_> = stream.try_collect().await.unwrap();

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 5);

    let names = batches[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(names.value(0), "alice");
}
