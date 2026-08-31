use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::query::UriRequest;
use crate::types;
use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    StringArray, TimestampMillisecondArray,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use commons::api::connections::DataConnectionResource;
use commons::api::connector::CredentialsResolver;
use commons::api::connector::{BinaryQuery, DataReader, FlightConnector, Query, QueryOptions, QueryOutput};
use commons::api::errors::ConnectorError;
use commons::utils::config::ConnectorConfig;
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

fn build_client(
    credentials: &HashMap<String, String>,
    connection_timeout: Duration,
) -> Result<UriClient, ConnectorError> {
    let raw_url = credentials
        .get(KEY_URI)
        .ok_or_else(|| ConnectorError::ConnectionError("'URI' credential is required".to_string()))?
        .clone();
    let mut base_url =
        url::Url::parse(&raw_url).map_err(|e| ConnectorError::ConnectionError(format!("Invalid URI: {e}")))?;
    base_url.set_query(None);
    base_url.set_fragment(None);
    if !base_url.path().ends_with('/') {
        let normalized = format!("{}/", base_url.path());
        base_url.set_path(&normalized);
    }

    let request_timeout = Duration::from_secs(connection_timeout.as_secs().max(10) * 3);
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(connection_timeout)
        .read_timeout(request_timeout)
        .timeout(request_timeout);

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

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
        credentials_resolver: &dyn CredentialsResolver,
    ) -> Result<Arc<dyn DataReader>, ConnectorError> {
        let connection_timeout = self.config.connection_timeout();
        let cache_key = data_connection.metadata.id.clone();

        let client = self
            .clients
            .try_get_with(cache_key, async {
                let credentials = credentials_resolver.resolve(data_connection).await?;
                build_client(&credentials, connection_timeout)
            })
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to get URI client: {e}")))?;

        Ok(Arc::new(UriReader {
            client,
            cached_response: tokio::sync::Mutex::new(None),
        }))
    }
}

// The Flight protocol calls schema() then read() within a single do_get;
// cached_response avoids a redundant HTTP round-trip for that pair.
// A separate get_flight_info call creates its own reader and will issue
// an additional request — this is inherent to the Flight two-phase flow
// and affects all connectors equally.
struct UriReader {
    client: UriClient,
    cached_response: tokio::sync::Mutex<Option<serde_json::Value>>,
}

const MAX_RESPONSE_BYTES: u64 = 128 * 1024 * 1024;

async fn fetch_json(client: &UriClient, request: &UriRequest) -> Result<serde_json::Value, ConnectorError> {
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

    serde_json::from_slice(&bytes)
        .map_err(|e| ConnectorError::ConnectionError(format!("Failed to parse JSON response: {e}")))
}

#[async_trait::async_trait]
impl DataReader for UriReader {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<Query>, ConnectorError> {
        let request = UriRequest::parse(query)?;
        let response_json = fetch_json(&self.client, &request).await?;
        let rows = types::extract_rows(&response_json, request.data_path.as_deref())?;

        if rows.is_empty() {
            return Err(ConnectorError::NoDataError);
        }

        let schema = types::infer_schema(rows);
        *self.cached_response.lock().await = Some(response_json);
        Ok(Arc::new(Query::new(query.to_owned(), Arc::new(schema))))
    }

    async fn read_tabular(&self, view: Arc<Query>, options: &QueryOptions) -> QueryOutput {
        let request = UriRequest::parse(&view.query)?;
        let schema = view.schema.clone();
        let client = self.client.clone();
        let batch_size = options.batch_size;
        let cached = self.cached_response.lock().await.take();

        let stream = async_stream::try_stream! {
            let response_json = match cached {
                Some(json) => json,
                None => fetch_json(&client, &request).await?,
            };
            let rows = types::extract_rows(&response_json, request.data_path.as_deref())?;

            for chunk in rows.chunks(batch_size) {
                let batch = rows_to_record_batch(&schema, chunk)?;
                yield batch;
            }
        };

        Ok(Box::pin(stream))
    }

    async fn can_read_binary(&self, query: Arc<BinaryQuery>) -> Result<(), ConnectorError> {
        let response = self
            .client
            .request(reqwest::Method::HEAD, &query.path)?
            .send()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("HTTP HEAD failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(ConnectorError::IOError(format!(
                "Cannot read '{}': HTTP {status}",
                query.path
            )));
        }
        Ok(())
    }

    async fn read_binary(&self, query: Arc<BinaryQuery>) -> QueryOutput {
        // Disable the total request deadline — binary downloads can be
        // arbitrarily large.  The client-level read_timeout still guards
        // against stalled connections (it resets on each received chunk).
        let response = self
            .client
            .request(reqwest::Method::GET, &query.path)?
            .timeout(Duration::from_secs(24 * 60 * 60))
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

fn rows_to_record_batch(schema: &Arc<Schema>, rows: &[serde_json::Value]) -> Result<RecordBatch, ConnectorError> {
    let arrays: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|field| {
            let values: Vec<Option<&serde_json::Value>> = rows
                .iter()
                .map(|row| row.get(field.name()).filter(|v| !v.is_null()))
                .collect();
            json_values_to_array(field.data_type(), &values)
        })
        .collect::<Result<_, _>>()?;

    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(|e| ConnectorError::SQLError(e.to_string()))
}

fn json_values_to_array(
    data_type: &ArrowDataType,
    values: &[Option<&serde_json::Value>],
) -> Result<ArrayRef, ConnectorError> {
    match data_type {
        ArrowDataType::Boolean => {
            let arr: BooleanArray = values.iter().map(|v| v.and_then(|v| v.as_bool())).collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int8 => {
            let arr: Int8Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_i64()).map(|n| n as i8))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int16 => {
            let arr: Int16Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_i64()).map(|n| n as i16))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int32 => {
            let arr: Int32Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_i64()).map(|n| n as i32))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int64 => {
            let arr: Int64Array = values.iter().map(|v| v.and_then(|v| v.as_i64())).collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Float32 => {
            let arr: Float32Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_f64()).map(|n| n as f32))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Float64 => {
            let arr: Float64Array = values.iter().map(|v| v.and_then(|v| v.as_f64())).collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Timestamp(TimeUnit::Millisecond, _) => {
            let arr: TimestampMillisecondArray = values
                .iter()
                .map(|v| {
                    v.and_then(|v| {
                        v.as_i64().or_else(|| {
                            v.as_str().and_then(|s| {
                                chrono::DateTime::parse_from_rfc3339(s)
                                    .ok()
                                    .map(|dt| dt.timestamp_millis())
                                    .or_else(|| {
                                        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                                            .ok()
                                            .map(|dt| dt.and_utc().timestamp_millis())
                                    })
                            })
                        })
                    })
                })
                .collect();
            Ok(Arc::new(arr.with_timezone("UTC")))
        },
        _ => {
            let arr: StringArray = values
                .iter()
                .map(|v| {
                    v.map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                })
                .collect();
            Ok(Arc::new(arr))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;
    use arrow::datatypes::Field;

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
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        assert_eq!(client.base_url.as_str(), "http://example.com/");
        assert!(matches!(client.auth, UriAuth::None));
    }

    #[test]
    fn test_build_client_token_auth() {
        let creds = HashMap::from([
            (KEY_URI.to_string(), "http://example.com".to_string()),
            (KEY_TOKEN.to_string(), "my-token".to_string()),
        ]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        assert!(matches!(client.auth, UriAuth::Token { .. }));
    }

    #[test]
    fn test_build_client_basic_auth() {
        let creds = HashMap::from([
            (KEY_URI.to_string(), "http://example.com".to_string()),
            (KEY_USERNAME.to_string(), "user".to_string()),
            (KEY_PASSWORD.to_string(), "pass".to_string()),
        ]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
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
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        assert!(matches!(client.auth, UriAuth::Token { .. }));
    }

    #[test]
    fn test_build_client_missing_uri() {
        let creds = HashMap::new();
        assert!(build_client(&creds, Duration::from_secs(10)).is_err());
    }

    #[test]
    fn test_url_join_relative_path() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com".to_string())]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        let req = client.request(reqwest::Method::GET, "api/data").unwrap();
        assert_eq!(req.build().unwrap().url().as_str(), "http://example.com/api/data");
    }

    #[test]
    fn test_url_join_trailing_slash_base() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com/".to_string())]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        let req = client.request(reqwest::Method::GET, "api/data").unwrap();
        assert_eq!(req.build().unwrap().url().as_str(), "http://example.com/api/data");
    }

    #[test]
    fn test_url_join_base_with_path_prefix() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com/v1".to_string())]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        let req = client.request(reqwest::Method::GET, "data").unwrap();
        assert_eq!(req.build().unwrap().url().as_str(), "http://example.com/v1/data");
    }

    #[test]
    fn test_url_rejects_absolute_url_with_scheme() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com".to_string())]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        let err = client
            .request(reqwest::Method::GET, "https://evil.com/steal")
            .unwrap_err();
        assert!(err.to_string().contains("escape the base URI"));
    }

    #[test]
    fn test_url_scheme_colon_treated_as_relative() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com".to_string())]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        let req = client.request(reqwest::Method::GET, "http:evil.com").unwrap();
        assert_eq!(req.build().unwrap().url().host_str(), Some("example.com"));
    }

    #[test]
    fn test_url_rejects_different_port() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com:8080".to_string())]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        let err = client
            .request(reqwest::Method::GET, "http://example.com:9090/x")
            .unwrap_err();
        assert!(err.to_string().contains("escape the base URI"));
    }

    #[test]
    fn test_url_rejects_path_escape() {
        let creds = HashMap::from([(KEY_URI.to_string(), "http://example.com/v1".to_string())]);
        let client = build_client(&creds, Duration::from_secs(10)).unwrap();
        let err = client
            .request(reqwest::Method::GET, "http://example.com/admin")
            .unwrap_err();
        assert!(err.to_string().contains("escape the base URI"));
    }

    #[test]
    fn test_json_values_to_array_boolean() {
        let v_true = serde_json::json!(true);
        let v_false = serde_json::json!(false);
        let vals = vec![Some(&v_true), None, Some(&v_false)];
        let arr = json_values_to_array(&ArrowDataType::Boolean, &vals).unwrap();
        let bool_arr = arr.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert_eq!(bool_arr.len(), 3);
        assert!(bool_arr.value(0));
        assert!(bool_arr.is_null(1));
        assert!(!bool_arr.value(2));
    }

    #[test]
    fn test_json_values_to_array_int64() {
        let v1 = serde_json::json!(42);
        let v2 = serde_json::json!(99);
        let vals = vec![Some(&v1), Some(&v2), None];
        let arr = json_values_to_array(&ArrowDataType::Int64, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(int_arr.value(0), 42);
        assert_eq!(int_arr.value(1), 99);
        assert!(int_arr.is_null(2));
    }

    #[test]
    fn test_json_values_to_array_float64() {
        let v = serde_json::json!(1.23);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array(&ArrowDataType::Float64, &vals).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((f_arr.value(0) - 1.23).abs() < f64::EPSILON);
        assert!(f_arr.is_null(1));
    }

    #[test]
    fn test_json_values_to_array_utf8_fallback() {
        let v_str = serde_json::json!("hello");
        let v_obj = serde_json::json!({"nested": true});
        let vals = vec![Some(&v_str), Some(&v_obj), None];
        let arr = json_values_to_array(&ArrowDataType::Utf8, &vals).unwrap();
        let str_arr = arr.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(str_arr.value(0), "hello");
        assert_eq!(str_arr.value(1), r#"{"nested":true}"#);
        assert!(str_arr.is_null(2));
    }

    #[test]
    fn test_json_values_to_array_timestamp_epoch() {
        let v = serde_json::json!(1700000000000_i64);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array(
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
        assert!(ts_arr.is_null(1));
    }

    #[test]
    fn test_json_values_to_array_timestamp_iso() {
        let v = serde_json::json!("2023-11-14T22:13:20.000Z");
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
    }

    #[test]
    fn test_rows_to_record_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, true),
            Field::new("value", ArrowDataType::Int64, true),
        ]));
        let rows = vec![
            serde_json::json!({"name": "a", "value": 1}),
            serde_json::json!({"name": "b", "value": 2}),
        ];
        let batch = rows_to_record_batch(&schema, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);

        let name_arr = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(name_arr.value(0), "a");
        assert_eq!(name_arr.value(1), "b");

        let val_arr = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(val_arr.value(0), 1);
        assert_eq!(val_arr.value(1), 2);
    }

    #[test]
    fn test_rows_to_record_batch_with_nulls() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, true),
            Field::new("value", ArrowDataType::Int64, true),
        ]));
        let rows = vec![
            serde_json::json!({"name": "a"}),
            serde_json::json!({"name": "b", "value": null}),
        ];
        let batch = rows_to_record_batch(&schema, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);

        let val_arr = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        assert!(val_arr.is_null(0));
        assert!(val_arr.is_null(1));
    }
}
