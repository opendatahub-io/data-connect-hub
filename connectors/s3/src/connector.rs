use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::format::{self, FileFormat};
use arrow::array::BinaryArray;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use commons::api::connections::DataConnectionResource;
use commons::api::connector::{BinaryQuery, CredentialsResolver};
use commons::api::connector::{DataReader, FlightConnector, Query, QueryOptions, QueryOutput, TableInfo};
use commons::api::errors::ConnectorError;
use commons::utils::config::ConnectorConfig;
use futures::TryStreamExt;
use moka::future::Cache;
use opendal::{EntryMode, Operator, Reader, layers::TimeoutLayer, services::S3};

const KEY_BUCKET: &str = "AWS_S3_BUCKET";
const KEY_ACCESS_KEY_ID: &str = "AWS_ACCESS_KEY_ID";
const KEY_SECRET_ACCESS_KEY: &str = "AWS_SECRET_ACCESS_KEY";
const KEY_REGION: &str = "AWS_DEFAULT_REGION";
const KEY_ENDPOINT: &str = "AWS_S3_ENDPOINT";

pub struct S3Connector {
    operators: Cache<String, Operator>,
    config: ConnectorConfig,
}

impl S3Connector {
    pub fn new(cache_ttl: Duration, cache_idle: Duration, cache_max_capacity: u64, config: ConnectorConfig) -> Self {
        Self {
            operators: Cache::builder()
                .time_to_live(cache_ttl)
                .time_to_idle(cache_idle)
                .max_capacity(cache_max_capacity)
                .build(),
            config,
        }
    }

    pub async fn insert_operator(&self, connection_id: &str, operator: Operator) {
        self.operators.insert(connection_id.to_string(), operator).await;
    }
}

fn build_operator(
    credentials: &HashMap<String, String>,
    connection_timeout: Duration,
) -> Result<Operator, ConnectorError> {
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

    let op = Operator::new(builder)
        .map_err(|e| ConnectorError::ConnectionError(format!("Failed to create S3 operator: {e}")))?
        .layer(TimeoutLayer::new().with_timeout(connection_timeout));
    Ok(op)
}

const PROVIDER: &str = "s3";

#[async_trait::async_trait]
impl FlightConnector for S3Connector {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    fn description(&self) -> String {
        "Amazon compatible S3 connector".to_string()
    }

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
        credentials_resolver: &dyn CredentialsResolver,
    ) -> Result<Arc<dyn DataReader>, ConnectorError> {
        let connection_timeout = self.config.connection_timeout();
        let cache_key = data_connection.metadata.id.clone();

        let operator = self
            .operators
            .try_get_with(cache_key, async {
                let credentials = credentials_resolver.resolve(data_connection).await?;
                build_operator(&credentials, connection_timeout)
            })
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to get S3 operator: {e}")))?;

        let format_hint = data_connection.resource.properties.get("format").cloned();
        Ok(Arc::new(S3Reader {
            operator,
            format_hint,
            config: self.config,
        }))
    }
}

pub struct S3Reader {
    operator: Operator,
    format_hint: Option<String>,
    config: ConnectorConfig,
}

impl S3Reader {
    fn detect_format(&self, path: &str) -> Result<FileFormat, ConnectorError> {
        FileFormat::detect(path, self.format_hint.as_deref())
    }

    async fn make_reader(&self, path: &str) -> Result<Reader, ConnectorError> {
        let reader = self
            .operator
            .reader_with(path)
            .chunk(self.config.chunk_size)
            .await
            .map_err(|e| ConnectorError::IOError(format!("Failed to create S3 reader for '{path}': {e}")))?;
        Ok(reader)
    }
}

#[async_trait::async_trait]
impl DataReader for S3Reader {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<Query>, ConnectorError> {
        let format = self.detect_format(query)?;
        let reader = self.make_reader(query).await?;

        let schema = match format {
            FileFormat::Parquet => format::read_parquet_schema(reader).await?,
            FileFormat::Csv => format::read_csv_schema(reader).await?,
            FileFormat::JsonLines => format::read_jsonl_schema(reader).await?,
        };

        Ok(Arc::new(Query::new(query.to_owned(), Arc::new(schema))))
    }

    async fn read_tabular(&self, view: Arc<Query>, options: &QueryOptions) -> QueryOutput {
        let format = self.detect_format(&view.query)?;
        let batch_size = options.batch_size;

        match format {
            FileFormat::Parquet => {
                let reader = self.make_reader(&view.query).await?;
                format::read_parquet_batches(reader, batch_size).await
            },
            FileFormat::Csv => {
                let reader = self.make_reader(&view.query).await?;
                format::read_csv_batches(reader, &view.schema, batch_size).await
            },
            FileFormat::JsonLines => {
                let reader = self.make_reader(&view.query).await?;
                format::read_jsonl_batches(reader, &view.schema, batch_size).await
            },
        }
    }

    async fn can_read_binary(&self, query: Arc<BinaryQuery>) -> Result<(), ConnectorError> {
        self.operator
            .stat(&query.path)
            .await
            .map_err(|e| ConnectorError::IOError(format!("Cannot read '{}': {e}", query.path)))?;
        Ok(())
    }

    async fn read_binary(&self, query: Arc<BinaryQuery>) -> QueryOutput {
        let reader = self.make_reader(&query.path).await?;
        let schema = Arc::new(Schema::new(vec![Field::new("data", DataType::Binary, false)]));

        let mut buf_stream = reader
            .into_stream(..)
            .await
            .map_err(|e| ConnectorError::IOError(format!("Failed to open stream for '{}': {e}", query.path)))?;

        let stream = async_stream::try_stream! {
            while let Some(buf) = buf_stream
                .try_next()
                .await
                .map_err(|e| ConnectorError::IOError(format!("Stream read error: {e}")))?
            {
                let chunk = buf.to_bytes();
                let array = BinaryArray::from_vec(vec![&chunk]);
                let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(array)])
                    .map_err(|e| ConnectorError::IOError(format!("Failed to create batch: {e}")))?;
                yield batch;
            }
        };

        Ok(Box::pin(stream))
    }

    async fn check_connection(&self) -> Result<(), ConnectorError> {
        self.operator
            .check()
            .await
            .map_err(|_| ConnectorError::ConnectionError("Failed to check S3 connection".to_string()))
    }

    async fn list_tables(
        &self,
        table_name_filter: Option<&str>,
        include_schema: bool,
    ) -> Result<Vec<TableInfo>, ConnectorError> {
        let entries = self
            .operator
            .list_with("")
            .recursive(true)
            .await
            .map_err(|e| ConnectorError::IOError(format!("Failed to list S3 objects: {e}")))?;

        let paths: Vec<String> = entries
            .into_iter()
            .filter(|e| e.metadata().mode() == EntryMode::FILE)
            .map(|e| e.path().to_string())
            .filter(|p| FileFormat::detect(p, self.format_hint.as_deref()).is_ok())
            .collect();

        let mut tables = Vec::new();
        for path in &paths {
            if let Some(pattern) = table_name_filter
                && !sql_like_match(path, pattern)
            {
                continue;
            }

            let table_schema = if include_schema {
                match self.schema(path).await {
                    Ok(state) => state.schema.as_ref().clone(),
                    Err(e) => {
                        tracing::warn!(path, error = %e, "failed to read schema");
                        Schema::empty()
                    },
                }
            } else {
                Schema::empty()
            };

            tables.push(TableInfo {
                catalog: String::new(),
                schema_name: String::new(),
                table_name: path.clone(),
                table_type: "TABLE".to_string(),
                table_schema,
            });
        }

        Ok(tables)
    }
}

fn sql_like_match(value: &str, pattern: &str) -> bool {
    let v: Vec<char> = value.chars().collect();
    let p: Vec<char> = pattern.chars().collect();

    let (mut vi, mut pi) = (0usize, 0usize);
    let (mut star_pi, mut star_vi): (Option<usize>, usize) = (None, 0);

    while vi < v.len() {
        if pi < p.len() && (p[pi] == '_' || p[pi] == v[vi]) {
            vi += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == '%' {
            star_pi = Some(pi);
            pi += 1;
            star_vi = vi;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_vi += 1;
            vi = star_vi;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '%' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;

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

    #[test]
    fn test_s3_connector_provider() {
        let connector = S3Connector::new(
            Duration::from_secs(300),
            Duration::from_secs(60),
            100,
            ConnectorConfig::default(),
        );
        assert_eq!(connector.provider(), "s3");
    }

    #[test]
    fn test_build_operator_success() {
        let creds = make_credentials();
        let result = build_operator(&creds, Duration::from_secs(10));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_operator_missing_bucket() {
        let creds = HashMap::from([
            (KEY_ACCESS_KEY_ID.to_string(), "key".to_string()),
            (KEY_SECRET_ACCESS_KEY.to_string(), "secret".to_string()),
        ]);
        let result = build_operator(&creds, Duration::from_secs(10));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(KEY_BUCKET));
    }

    #[test]
    fn test_build_operator_missing_access_key() {
        let creds = HashMap::from([
            (KEY_BUCKET.to_string(), "bucket".to_string()),
            (KEY_SECRET_ACCESS_KEY.to_string(), "secret".to_string()),
        ]);
        let result = build_operator(&creds, Duration::from_secs(10));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(KEY_ACCESS_KEY_ID));
    }

    #[test]
    fn test_build_operator_with_endpoint() {
        let mut creds: HashMap<String, String> = (*make_credentials()).clone();
        creds.insert(KEY_ENDPOINT.to_string(), "http://minio:9000".to_string());
        let result = build_operator(&creds, Duration::from_secs(10));
        assert!(result.is_ok());
    }

    #[test]
    fn test_s3_reader_detect_format() {
        let reader = S3Reader {
            operator: build_operator(&make_credentials(), Duration::from_secs(10)).unwrap(),
            format_hint: None,
            config: ConnectorConfig::default(),
        };
        assert_eq!(reader.detect_format("data/file.parquet").unwrap(), FileFormat::Parquet);
        assert_eq!(reader.detect_format("data/file.csv").unwrap(), FileFormat::Csv);
        assert_eq!(reader.detect_format("data/file.jsonl").unwrap(), FileFormat::JsonLines);
        assert_eq!(reader.detect_format("data/file.ndjson").unwrap(), FileFormat::JsonLines);
    }

    #[test]
    fn test_s3_reader_detect_format_with_hint() {
        let reader = S3Reader {
            operator: build_operator(&make_credentials(), Duration::from_secs(10)).unwrap(),
            format_hint: Some("parquet".to_string()),
            config: ConnectorConfig::default(),
        };
        assert_eq!(reader.detect_format("data/no-extension").unwrap(), FileFormat::Parquet);

        let reader = S3Reader {
            operator: build_operator(&make_credentials(), Duration::from_secs(10)).unwrap(),
            format_hint: Some("jsonl".to_string()),
            config: ConnectorConfig::default(),
        };
        assert_eq!(
            reader.detect_format("data/no-extension").unwrap(),
            FileFormat::JsonLines
        );
    }

    #[test]
    fn test_sql_like_exact() {
        assert!(sql_like_match("cities.parquet", "cities.parquet"));
        assert!(!sql_like_match("cities.parquet", "cities.csv"));
    }

    #[test]
    fn test_sql_like_percent() {
        assert!(sql_like_match("data/cities.parquet", "%cities%"));
        assert!(sql_like_match("cities.parquet", "%"));
        assert!(sql_like_match("data/cities.parquet", "data/%"));
        assert!(!sql_like_match("data/cities.parquet", "other/%"));
    }

    #[test]
    fn test_sql_like_underscore() {
        assert!(sql_like_match("a.csv", "_.csv"));
        assert!(!sql_like_match("ab.csv", "_.csv"));
    }

    #[test]
    fn test_sql_like_combined() {
        assert!(!sql_like_match("data/cities.parquet", "%/_.parquet"));
        assert!(sql_like_match("data/x.parquet", "%/_.parquet"));
    }

    #[tokio::test]
    async fn test_can_read_binary_exists() {
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        op.write("data/model.bin", vec![1u8, 2, 3]).await.unwrap();
        let reader = S3Reader {
            operator: op,
            format_hint: None,
            config: ConnectorConfig::default(),
        };
        let query = Arc::new(BinaryQuery::new("data/model.bin".to_string()));
        assert!(reader.can_read_binary(query).await.is_ok());
    }

    #[tokio::test]
    async fn test_can_read_binary_not_found() {
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        let reader = S3Reader {
            operator: op,
            format_hint: None,
            config: ConnectorConfig::default(),
        };
        let query = Arc::new(BinaryQuery::new("does/not/exist.bin".to_string()));
        assert!(reader.can_read_binary(query).await.is_err());
    }

    #[tokio::test]
    async fn test_read_binary_streams_data() {
        let data = b"hello binary world".to_vec();
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        op.write("test.bin", data.clone()).await.unwrap();
        let reader = S3Reader {
            operator: op,
            format_hint: None,
            config: ConnectorConfig::default(),
        };
        let query = Arc::new(BinaryQuery::new("test.bin".to_string()));
        let stream = reader.read_binary(query).await.unwrap();

        let batches: Vec<_> = stream.try_collect().await.unwrap();
        assert!(!batches.is_empty());

        let mut result = Vec::new();
        for batch in &batches {
            assert_eq!(batch.num_columns(), 1);
            let col = batch.column(0).as_any().downcast_ref::<BinaryArray>().unwrap();
            for i in 0..col.len() {
                result.extend_from_slice(col.value(i));
            }
        }
        assert_eq!(result, data);
    }
}
