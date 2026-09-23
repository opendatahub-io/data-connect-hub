use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::query::UriRequest;
use arrow::array::BinaryArray;
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use commons::api::connections::DataConnectionResource;
use commons::api::connector::CredentialsResolver;
use commons::api::connector::{BinaryQuery, DataReader, FlightConnector, Query, QueryOptions, QueryOutput};
use commons::api::errors::ConnectorError;
use commons::utils::config::ConnectorConfig;
use format_readers::FileFormat;
use moka::future::Cache;

const KEY_URI: &str = "URI";
const KEY_TOKEN: &str = "TOKEN";
const KEY_USERNAME: &str = "USERNAME";
const KEY_PASSWORD: &str = "PASSWORD";
const KEY_CA_CERT: &str = "CA_CERT";

#[derive(Clone)]
struct UriClient {
    http: reqwest::Client,
    base_url: url::Url,
    auth: UriAuth,
}

#[derive(Clone)]
enum UriAuth {
    None,
    Token { token: String },
    Basic { username: String, password: String },
}

impl UriClient {
    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder, ConnectorError> {
        let url = self
            .base_url
            .join(path)
            .map_err(|e| ConnectorError::InvalidRequest(format!("Invalid path '{path}': {e}")))?;
        if url.origin() != self.base_url.origin() || !url.path().starts_with(self.base_url.path()) {
            return Err(ConnectorError::InvalidRequest(format!(
                "Path '{path}' must not escape the base URI"
            )));
        }
        let mut req = self.http.request(method, url);
        match &self.auth {
            UriAuth::None => {},
            UriAuth::Token { token } => {
                req = req.bearer_auth(token);
            },
            UriAuth::Basic { username, password } => {
                req = req.basic_auth(username, Some(password));
            },
        }
        Ok(req)
    }
}

pub struct UriConnector {
    clients: Cache<String, UriClient>,
    config: ConnectorConfig,
}

impl UriConnector {
    pub fn new(cache_ttl: Duration, cache_idle: Duration, cache_max_capacity: u64, config: ConnectorConfig) -> Self {
        Self {
            clients: Cache::builder()
                .time_to_live(cache_ttl)
                .time_to_idle(cache_idle)
                .max_capacity(cache_max_capacity)
                .build(),
            config,
        }
    }
}

fn build_client(credentials: &HashMap<String, String>, config: ConnectorConfig) -> Result<UriClient, ConnectorError> {
    let raw_url = credentials
        .get(KEY_URI)
        .cloned()
        .ok_or_else(|| ConnectorError::ConnectionError(format!("'{KEY_URI}' credential is required")))?;
    let mut base_url =
        url::Url::parse(&raw_url).map_err(|e| ConnectorError::ConnectionError(format!("Invalid URI: {e}")))?;
    base_url.set_query(None);
    base_url.set_fragment(None);
    if !base_url.path().ends_with('/') {
        let normalized = format!("{}/", base_url.path());
        base_url.set_path(&normalized);
    }

    let mut builder = reqwest::Client::builder()
        .connect_timeout(config.connection_timeout())
        .read_timeout(config.read_timeout())
        .timeout(config.request_timeout());

    if let Some(ca_pem) = credentials.get(KEY_CA_CERT) {
        let cert = reqwest::tls::Certificate::from_pem(ca_pem.as_bytes())
            .map_err(|e| ConnectorError::ConnectionError(format!("Invalid CA certificate: {e}")))?;
        builder = builder.add_root_certificate(cert);
    }

    let http = builder
        .build()
        .map_err(|e| ConnectorError::ConnectionError(format!("Failed to build HTTP client: {e}")))?;

    let auth = if let Some(token) = credentials.get(KEY_TOKEN) {
        UriAuth::Token { token: token.clone() }
    } else if let (Some(username), Some(password)) = (credentials.get(KEY_USERNAME), credentials.get(KEY_PASSWORD)) {
        UriAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }
    } else {
        UriAuth::None
    };

    Ok(UriClient { http, base_url, auth })
}

const PROVIDER: &str = "uri";

#[async_trait::async_trait]
impl FlightConnector for UriConnector {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    fn description(&self) -> String {
        "URI connector".to_string()
    }

    #[tracing::instrument(
        skip_all,
        fields(
        connector.provider = PROVIDER,
        connection.id = %data_connection.metadata.id,
        )
    )]
    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
        credentials_resolver: &dyn CredentialsResolver,
    ) -> Result<Arc<dyn DataReader>, ConnectorError> {
        let cache_key = data_connection.metadata.id.clone();

        let client = self
            .clients
            .try_get_with(cache_key, async {
                let credentials = credentials_resolver.resolve(data_connection).await?;
                build_client(&credentials, self.config)
            })
            .await
            .map_err(|e| Arc::try_unwrap(e).unwrap_or_else(|arc| (*arc).clone()))?;

        Ok(Arc::new(UriReader {
            client,
            cached_response: tokio::sync::Mutex::new(None),
        }))
    }
}

struct UriReader {
    client: UriClient,
    cached_response: tokio::sync::Mutex<Option<CachedResponse>>,
}

struct CachedResponse {
    format: FileFormat,
    bytes: bytes::Bytes,
}

const MAX_RESPONSE_BYTES: u64 = 128 * 1024 * 1024;

fn detect_format(
    content_type: Option<&reqwest::header::HeaderValue>,
    request: &UriRequest,
) -> Result<FileFormat, ConnectorError> {
    if let Some(fmt) = &request.format {
        return FileFormat::from_format_str(fmt);
    }

    if let Some(ct) = content_type {
        let ct_str = ct
            .to_str()
            .map_err(|e| ConnectorError::ConnectionError(format!("Invalid HTTP Content-Type header: {e}")))?;
        if let Ok(format) = FileFormat::from_content_type(ct_str) {
            return Ok(format);
        }
    }

    if let Ok(format) = FileFormat::detect(&request.path, None) {
        return Ok(format);
    }

    Err(ConnectorError::ConnectionError(
        "Cannot determine response format. Set 'format' in query or ensure the server returns a recognized Content-Type header.".to_string(),
    ))
}

async fn fetch_response(client: &UriClient, request: &UriRequest) -> Result<CachedResponse, ConnectorError> {
    let response = client
        .request(reqwest::Method::GET, &request.path)?
        .send()
        .await
        .map_err(|e| ConnectorError::ConnectionError(format!("HTTP request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ConnectorError::ConnectionError(format!(
            "HTTP request failed (status {status}): {body}"
        )));
    }

    let format = detect_format(response.headers().get(reqwest::header::CONTENT_TYPE), request)?;

    if let Some(len) = response.content_length()
        && len > MAX_RESPONSE_BYTES
    {
        return Err(ConnectorError::ConnectionError(format!(
            "Response too large ({len} bytes, limit {MAX_RESPONSE_BYTES})"
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| ConnectorError::ConnectionError(format!("Failed to read response: {e}")))?;

    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(ConnectorError::ConnectionError(format!(
            "Response too large ({} bytes, limit {MAX_RESPONSE_BYTES})",
            bytes.len()
        )));
    }

    Ok(CachedResponse { format, bytes })
}

async fn bytes_to_opendal_reader(data: bytes::Bytes) -> Result<opendal::Reader, ConnectorError> {
    let op = opendal::Operator::new(opendal::services::Memory::default())
        .map_err(|e| ConnectorError::IOError(format!("Failed to create memory operator: {e}")))?;
    op.write("data", data)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to buffer response: {e}")))?;
    op.reader("data")
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to create reader: {e}")))
}

#[async_trait::async_trait]
impl DataReader for UriReader {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER))]
    async fn schema(&self, query: &str) -> Result<Arc<Query>, ConnectorError> {
        let request = UriRequest::parse(query)?;
        let cached = fetch_response(&self.client, &request).await?;

        let schema = match cached.format {
            FileFormat::Json => format_readers::read_json_schema(&cached.bytes, request.data_path.as_deref())?,
            FileFormat::Csv => {
                let reader = bytes_to_opendal_reader(cached.bytes.clone()).await?;
                format_readers::read_csv_schema(reader).await?
            },
            FileFormat::JsonLines => {
                let reader = bytes_to_opendal_reader(cached.bytes.clone()).await?;
                format_readers::read_jsonl_schema(reader).await?
            },
            FileFormat::Parquet => {
                let reader = bytes_to_opendal_reader(cached.bytes.clone()).await?;
                format_readers::read_parquet_schema(reader).await?
            },
        };

        *self.cached_response.lock().await = Some(cached);
        Ok(Arc::new(Query::new(query.to_owned(), Arc::new(schema))))
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER, batch_size = options.batch_size))]
    async fn read_tabular(&self, view: Arc<Query>, options: &QueryOptions) -> QueryOutput {
        let request = UriRequest::parse(&view.query)?;
        let schema = view.schema.clone();
        let client = self.client.clone();
        let batch_size = options.batch_size;
        let cached = self.cached_response.lock().await.take();

        let resp = match cached {
            Some(c) => c,
            None => fetch_response(&client, &request).await?,
        };

        match resp.format {
            FileFormat::Json => {
                format_readers::read_json_batches(resp.bytes.to_vec(), schema, batch_size, request.data_path.as_deref())
            },
            FileFormat::Csv => {
                let reader = bytes_to_opendal_reader(resp.bytes).await?;
                format_readers::read_csv_batches(reader, &schema, batch_size).await
            },
            FileFormat::JsonLines => {
                let reader = bytes_to_opendal_reader(resp.bytes).await?;
                format_readers::read_jsonl_batches(reader, &schema, batch_size).await
            },
            FileFormat::Parquet => {
                let reader = bytes_to_opendal_reader(resp.bytes).await?;
                format_readers::read_parquet_batches(reader, batch_size).await
            },
        }
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER, storage.object.path = %query.path))]
    async fn can_read_binary(&self, query: Arc<BinaryQuery>) -> Result<(), ConnectorError> {
        let response = self
            .client
            .request(reqwest::Method::HEAD, &query.path)?
            .send()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("HTTP HEAD failed: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ConnectorError::NotFound(format!("'{}' not found", query.path)));
        }
        if !status.is_success() {
            return Err(ConnectorError::IOError(format!(
                "Cannot read '{}': HTTP {status}",
                query.path
            )));
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER, storage.object.path = %query.path))]
    async fn read_binary(&self, query: Arc<BinaryQuery>) -> QueryOutput {
        let response = self
            .client
            .request(reqwest::Method::GET, &query.path)?
            .timeout(Duration::from_secs(24 * 60 * 60))
            .send()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ConnectorError::NotFound(format!("'{}' not found", query.path)));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ConnectorError::ConnectionError(format!(
                "HTTP request failed (status {status}): {body}"
            )));
        }

        let schema = Arc::new(Schema::new(vec![Field::new("data", ArrowDataType::Binary, false)]));

        let mut byte_stream = response.bytes_stream();

        let stream = async_stream::try_stream! {
            use futures::StreamExt;
            while let Some(result) = byte_stream.next().await {
                let chunk = result
                    .map_err(|e| ConnectorError::IOError(format!("Stream read error: {e}")))?;
                let data: &[u8] = &chunk;
                let array = BinaryArray::from_vec(vec![data]);
                let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(array)])
                    .map_err(|e| ConnectorError::IOError(format!("Failed to create batch: {e}")))?;
                yield batch;
            }
        };

        Ok(Box::pin(stream))
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER))]
    async fn check_connection(&self) -> Result<(), ConnectorError> {
        let response = self
            .client
            .request(reqwest::Method::HEAD, "")?
            .send()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Connection test failed: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ConnectorError::ConnectionError(format!(
                "Authentication failed (HTTP {status})"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn test_connector_provider() {
        let connector = UriConnector::new(
            Duration::from_secs(300),
            Duration::from_secs(60),
            100,
            ConnectorConfig::default(),
        );
        assert_eq!(connector.provider(), "uri");
    }

    #[test]
    fn test_build_client_no_auth() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        assert_eq!(client.base_url.as_str(), "http://example.com/");
        assert!(matches!(client.auth, UriAuth::None));
    }

    #[test]
    fn test_build_client_token_auth() {
        let creds = HashMap::from([
            (KEY_URI.to_string(), "http://example.com".to_string()),
            (KEY_TOKEN.to_string(), "my-token".to_string()),
        ]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        assert!(matches!(client.auth, UriAuth::Token { .. }));
    }

    #[test]
    fn test_build_client_basic_auth() {
        let creds = HashMap::from([
            (KEY_URI.to_string(), "http://example.com".to_string()),
            (KEY_USERNAME.to_string(), "user".to_string()),
            (KEY_PASSWORD.to_string(), "pass".to_string()),
        ]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        assert!(matches!(client.auth, UriAuth::Basic { .. }));
    }

    #[test]
    fn test_build_client_token_takes_precedence() {
        let creds = HashMap::from([
            (KEY_URI.to_string(), "http://example.com".to_string()),
            (KEY_TOKEN.to_string(), "my-token".to_string()),
            (KEY_USERNAME.to_string(), "user".to_string()),
            (KEY_PASSWORD.to_string(), "pass".to_string()),
        ]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        assert!(matches!(client.auth, UriAuth::Token { .. }));
    }

    #[test]
    fn test_build_client_missing_uri() {
        let creds = HashMap::new();
        assert!(build_client(&creds, ConnectorConfig::default()).is_err());
    }

    #[test]
    fn test_detect_format_from_content_type_json() {
        let request = UriRequest {
            path: "api/data".to_string(),
            data_path: None,
            format: None,
        };
        let ct = HeaderValue::from_static("application/json; charset=utf-8");
        assert_eq!(detect_format(Some(&ct), &request).unwrap(), FileFormat::Json);
    }

    #[test]
    fn test_detect_format_from_content_type_json_suffix() {
        let request = UriRequest {
            path: "api/data".to_string(),
            data_path: None,
            format: None,
        };
        let ct = HeaderValue::from_static("application/problem+json");
        assert_eq!(detect_format(Some(&ct), &request).unwrap(), FileFormat::Json);
    }

    #[test]
    fn test_detect_format_from_content_type_csv() {
        let request = UriRequest {
            path: "api/data".to_string(),
            data_path: None,
            format: None,
        };
        let ct = HeaderValue::from_static("text/csv");
        assert_eq!(detect_format(Some(&ct), &request).unwrap(), FileFormat::Csv);
    }

    #[test]
    fn test_detect_format_from_content_type_ndjson() {
        let request = UriRequest {
            path: "api/data".to_string(),
            data_path: None,
            format: None,
        };
        let ct = HeaderValue::from_static("application/x-ndjson");
        assert_eq!(detect_format(Some(&ct), &request).unwrap(), FileFormat::JsonLines);
    }

    #[test]
    fn test_detect_format_from_content_type_parquet() {
        let request = UriRequest {
            path: "api/data".to_string(),
            data_path: None,
            format: None,
        };
        let ct = HeaderValue::from_static("application/vnd.apache.parquet");
        assert_eq!(detect_format(Some(&ct), &request).unwrap(), FileFormat::Parquet);
    }

    #[test]
    fn test_detect_format_explicit_overrides_content_type() {
        let request = UriRequest {
            path: "api/data".to_string(),
            data_path: None,
            format: Some("csv".to_string()),
        };
        let ct = HeaderValue::from_static("application/json");
        assert_eq!(detect_format(Some(&ct), &request).unwrap(), FileFormat::Csv);
    }

    #[test]
    fn test_detect_format_falls_back_to_path_extension() {
        let request = UriRequest {
            path: "data/file.parquet".to_string(),
            data_path: None,
            format: None,
        };
        let ct = HeaderValue::from_static("application/octet-stream");
        assert_eq!(detect_format(Some(&ct), &request).unwrap(), FileFormat::Parquet);
    }

    #[test]
    fn test_detect_format_no_content_type_uses_path() {
        let request = UriRequest {
            path: "data/file.csv".to_string(),
            data_path: None,
            format: None,
        };
        assert_eq!(detect_format(None, &request).unwrap(), FileFormat::Csv);
    }

    #[test]
    fn test_detect_format_fails_when_ambiguous() {
        let request = UriRequest {
            path: "api/data".to_string(),
            data_path: None,
            format: None,
        };
        let ct = HeaderValue::from_static("application/octet-stream");
        assert!(detect_format(Some(&ct), &request).is_err());
    }

    #[test]
    fn test_url_join_relative_path() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        let req = client.request(reqwest::Method::GET, "api/data").unwrap();
        assert_eq!(req.build().unwrap().url().as_str(), "http://example.com/api/data");
    }

    #[test]
    fn test_url_join_trailing_slash_base() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com/".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        let req = client.request(reqwest::Method::GET, "api/data").unwrap();
        assert_eq!(req.build().unwrap().url().as_str(), "http://example.com/api/data");
    }

    #[test]
    fn test_url_join_base_with_path_prefix() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com/v1".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        let req = client.request(reqwest::Method::GET, "data").unwrap();
        assert_eq!(req.build().unwrap().url().as_str(), "http://example.com/v1/data");
    }

    #[test]
    fn test_url_rejects_absolute_url_with_scheme() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        let err = client
            .request(reqwest::Method::GET, "https://evil.com/steal")
            .unwrap_err();
        assert!(err.to_string().contains("escape the base URI"));
    }

    #[test]
    fn test_url_scheme_colon_treated_as_relative() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        let req = client.request(reqwest::Method::GET, "http:evil.com").unwrap();
        assert_eq!(req.build().unwrap().url().host_str(), Some("example.com"));
    }

    #[test]
    fn test_url_rejects_different_port() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com:8080".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        let err = client
            .request(reqwest::Method::GET, "http://example.com:9090/x")
            .unwrap_err();
        assert!(err.to_string().contains("escape the base URI"));
    }

    #[test]
    fn test_url_rejects_path_escape() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com/v1".to_string())]);
        let client = build_client(&creds, ConnectorConfig::default()).unwrap();
        let err = client
            .request(reqwest::Method::GET, "http://example.com/admin")
            .unwrap_err();
        assert!(err.to_string().contains("escape the base URI"));
    }
}
