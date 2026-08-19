use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::errors::ConnectorError;
use commons::api::tabular::{FlightConnector, QueryOptions, QueryOutput, TabularReader, TabularState};
use moka::future::Cache;
use opendal::{Operator, services::S3};

use crate::format::{self, FileFormat};

const KEY_BUCKET: &str = "AWS_S3_BUCKET";
const KEY_ACCESS_KEY_ID: &str = "AWS_ACCESS_KEY_ID";
const KEY_SECRET_ACCESS_KEY: &str = "AWS_SECRET_ACCESS_KEY";
const KEY_REGION: &str = "AWS_DEFAULT_REGION";
const KEY_ENDPOINT: &str = "AWS_S3_ENDPOINT";

pub struct S3Connector {
    operators: Cache<String, Operator>,
}

impl S3Connector {
    pub fn new(cache_ttl: Duration, cache_idle: Duration, cache_max_capacity: u64) -> Self {
        Self {
            operators: Cache::builder()
                .time_to_live(cache_ttl)
                .time_to_idle(cache_idle)
                .max_capacity(cache_max_capacity)
                .build(),
        }
    }

    pub async fn insert_operator(&self, connection_id: &str, operator: Operator) {
        self.operators.insert(connection_id.to_string(), operator).await;
    }
}

fn extract_credentials(
    data_connection: &DataConnectionResource,
) -> Result<Arc<HashMap<String, String>>, ConnectorError> {
    match &data_connection.resource.admin {
        Some(Admin::Secret { name: _, secret }) => Ok(secret.clone()),
        _ => Err(ConnectorError::ConnectionError(
            "S3 credentials are required".to_string(),
        )),
    }
}

fn build_operator(credentials: &HashMap<String, String>) -> Result<Operator, ConnectorError> {
    let bucket = credentials
        .get(KEY_BUCKET)
        .ok_or_else(|| ConnectorError::ConfigError(format!("{KEY_BUCKET} is required")))?;

    let access_key = credentials
        .get(KEY_ACCESS_KEY_ID)
        .ok_or_else(|| ConnectorError::ConfigError(format!("{KEY_ACCESS_KEY_ID} is required")))?;

    let secret_key = credentials
        .get(KEY_SECRET_ACCESS_KEY)
        .ok_or_else(|| ConnectorError::ConfigError(format!("{KEY_SECRET_ACCESS_KEY} is required")))?;

    let mut builder = S3::default()
        .bucket(bucket)
        .access_key_id(access_key)
        .secret_access_key(secret_key);

    if let Some(region) = credentials.get(KEY_REGION) {
        builder = builder.region(region);
    }

    if let Some(endpoint) = credentials.get(KEY_ENDPOINT)
        && !endpoint.is_empty()
    {
        builder = builder.endpoint(endpoint);
    }

    Operator::new(builder).map_err(|e| ConnectorError::ConnectionError(format!("Failed to create S3 operator: {e}")))
}

#[async_trait::async_trait]
impl FlightConnector for S3Connector {
    fn provider(&self) -> String {
        "s3".to_string()
    }

    fn description(&self) -> String {
        "Amazon compatible S3 connector".to_string()
    }

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
    ) -> Result<Arc<dyn TabularReader>, ConnectorError> {
        let credentials = extract_credentials(data_connection)?;
        let operator = self
            .operators
            .try_get_with_by_ref(&data_connection.metadata.id, async { build_operator(&credentials) })
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to get S3 operator: {e}")))?;

        let format_hint = data_connection.resource.properties.get("format").cloned();
        Ok(Arc::new(S3Reader { operator, format_hint }))
    }
}

pub struct S3Reader {
    operator: Operator,
    format_hint: Option<String>,
}

impl S3Reader {
    async fn read_object(&self, path: &str) -> Result<Bytes, ConnectorError> {
        let data = self
            .operator
            .read(path)
            .await
            .map_err(|e| ConnectorError::IOError(format!("Failed to read S3 object '{path}': {e}")))?
            .to_bytes();
        if data.is_empty() {
            return Err(ConnectorError::NoDataError);
        }
        Ok(data)
    }

    fn detect_format(&self, path: &str) -> Result<FileFormat, ConnectorError> {
        FileFormat::detect(path, self.format_hint.as_deref())
    }
}

#[async_trait::async_trait]
impl TabularReader for S3Reader {
    fn provider(&self) -> String {
        "s3".to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<TabularState>, ConnectorError> {
        let format = self.detect_format(query)?;
        let data = self.read_object(query).await?;

        let schema = match format {
            FileFormat::Parquet => format::read_parquet_schema(&data)?,
            FileFormat::Csv => format::read_csv_schema(&data)?,
            FileFormat::JsonLines => format::read_jsonl_schema(&data)?,
        };

        Ok(Arc::new(TabularState::new(query.to_owned(), Arc::new(schema))))
    }

    async fn read(&self, state: Arc<TabularState>, options: &QueryOptions) -> QueryOutput {
        let format = self.detect_format(&state.query)?;
        let data = self.read_object(&state.query).await?;
        let schema = state.schema.clone();
        let batch_size = options.batch_size;

        let iter = match format {
            FileFormat::Parquet => format::read_parquet_batches(data, batch_size)?,
            FileFormat::Csv => format::read_csv_batches(data, &schema, batch_size)?,
            FileFormat::JsonLines => format::read_jsonl_batches(data, &schema, batch_size)?,
        };

        let stream = async_stream::try_stream! {
            for batch in iter {
                yield batch?;
            }
        };

        Ok(Box::pin(stream))
    }

    async fn test_connection(&self) -> Result<(), ConnectorError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commons::api::ResourceMetadata;
    use commons::api::connections::{DataConnection, DataFormat};

    fn make_credentials() -> Arc<HashMap<String, String>> {
        Arc::new(HashMap::from([
            (KEY_BUCKET.to_string(), "test-bucket".to_string()),
            (KEY_ACCESS_KEY_ID.to_string(), "AKIAIOSFODNN7EXAMPLE".to_string()),
            (
                KEY_SECRET_ACCESS_KEY.to_string(),
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            ),
            (KEY_REGION.to_string(), "us-east-1".to_string()),
        ]))
    }

    fn make_connection(credentials: Arc<HashMap<String, String>>) -> DataConnectionResource {
        DataConnectionResource {
            metadata: ResourceMetadata {
                id: "conn-1".to_string(),
                tenant_id: Some("tenant-1".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: DataConnection {
                name: "test-s3".to_string(),
                data_connection_type_id: "s3-type".to_string(),
                format: DataFormat::Tabular,
                admin: Some(Admin::Secret {
                    name: "test-s3".to_string(),
                    secret: credentials,
                }),
                properties: HashMap::new(),
            },
            status: Default::default(),
        }
    }

    #[test]
    fn test_s3_connector_provider() {
        let connector = S3Connector::new(Duration::from_secs(300), Duration::from_secs(60), 100);
        assert_eq!(connector.provider(), "s3");
    }

    #[test]
    fn test_build_operator_success() {
        let creds = make_credentials();
        let result = build_operator(&creds);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_operator_missing_bucket() {
        let creds = HashMap::from([
            (KEY_ACCESS_KEY_ID.to_string(), "key".to_string()),
            (KEY_SECRET_ACCESS_KEY.to_string(), "secret".to_string()),
        ]);
        let result = build_operator(&creds);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(KEY_BUCKET));
    }

    #[test]
    fn test_build_operator_missing_access_key() {
        let creds = HashMap::from([
            (KEY_BUCKET.to_string(), "bucket".to_string()),
            (KEY_SECRET_ACCESS_KEY.to_string(), "secret".to_string()),
        ]);
        let result = build_operator(&creds);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(KEY_ACCESS_KEY_ID));
    }

    #[test]
    fn test_build_operator_with_endpoint() {
        let mut creds: HashMap<String, String> = (*make_credentials()).clone();
        creds.insert(KEY_ENDPOINT.to_string(), "http://minio:9000".to_string());
        let result = build_operator(&creds);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_credentials_success() {
        let creds = make_credentials();
        let conn = make_connection(creds.clone());
        let result = extract_credentials(&conn);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get(KEY_BUCKET).unwrap(), "test-bucket");
    }

    #[test]
    fn test_extract_credentials_missing() {
        let mut conn = make_connection(make_credentials());
        conn.resource.admin = None;
        let result = extract_credentials(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_credentials_secret_ref() {
        let mut conn = make_connection(make_credentials());
        conn.resource.admin = Some(Admin::SecretRef {
            secret_ref: "some-ref".to_string(),
        });
        let result = extract_credentials(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_s3_reader_detect_format() {
        let reader = S3Reader {
            operator: build_operator(&make_credentials()).unwrap(),
            format_hint: None,
        };
        assert_eq!(reader.detect_format("data/file.parquet").unwrap(), FileFormat::Parquet);
        assert_eq!(reader.detect_format("data/file.csv").unwrap(), FileFormat::Csv);
        assert_eq!(reader.detect_format("data/file.jsonl").unwrap(), FileFormat::JsonLines);
        assert_eq!(reader.detect_format("data/file.ndjson").unwrap(), FileFormat::JsonLines);
    }

    #[test]
    fn test_s3_reader_detect_format_with_hint() {
        let reader = S3Reader {
            operator: build_operator(&make_credentials()).unwrap(),
            format_hint: Some("parquet".to_string()),
        };
        assert_eq!(reader.detect_format("data/no-extension").unwrap(), FileFormat::Parquet);

        let reader = S3Reader {
            operator: build_operator(&make_credentials()).unwrap(),
            format_hint: Some("jsonl".to_string()),
        };
        assert_eq!(
            reader.detect_format("data/no-extension").unwrap(),
            FileFormat::JsonLines
        );
    }
}
