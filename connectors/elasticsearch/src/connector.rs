use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array, StringArray,
    TimestampMillisecondArray,
};
use arrow::datatypes::{DataType as ArrowDataType, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use commons::api::connection_types::Provider;
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::errors::ConnectorError;
use commons::api::tabular::{FlightConnector, QueryOptions, QueryOutput, TabularReader, TabularState};
use commons::utils::config::ConnectorConfig;
use moka::future::Cache;

use crate::query::EsRequestInput;
use crate::types;

const KEY_HOST: &str = "ES_HOST";
const KEY_USERNAME: &str = "ES_USERNAME";
const KEY_PASSWORD: &str = "ES_PASSWORD";
const KEY_API_KEY: &str = "ES_API_KEY";
const KEY_CA_CERT: &str = "ES_CA_CERT";

#[derive(Clone)]
struct EsClient {
    http: reqwest::Client,
    base_url: String,
    auth: EsAuth,
}

#[derive(Clone)]
enum EsAuth {
    None,
    Basic { username: String, password: String },
    ApiKey { encoded: String },
}

impl EsClient {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = self.http.request(method, &url);
        match &self.auth {
            EsAuth::None => {},
            EsAuth::Basic { username, password } => {
                req = req.basic_auth(username, Some(password));
            },
            EsAuth::ApiKey { encoded } => {
                req = req.header("Authorization", format!("ApiKey {encoded}"));
            },
        }
        req
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::GET, path)
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::POST, path)
    }

    fn delete(&self, path: &str) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::DELETE, path)
    }
}

pub struct ElasticsearchConnector {
    clients: Cache<String, EsClient>,
    config: ConnectorConfig,
}

impl ElasticsearchConnector {
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

fn extract_credentials(
    data_connection: &DataConnectionResource,
) -> Result<Arc<HashMap<String, String>>, ConnectorError> {
    match &data_connection.resource.admin {
        Some(Admin::Secret { name: _, secret }) => Ok(secret.clone()),
        _ => Err(ConnectorError::ConnectionError(
            "Elasticsearch credentials are required".to_string(),
        )),
    }
}

fn build_client(
    credentials: &HashMap<String, String>,
    connection_timeout: Duration,
) -> Result<EsClient, ConnectorError> {
    let base_url = credentials
        .get(KEY_HOST)
        .ok_or_else(|| ConnectorError::ConnectionError("Elasticsearch 'ES_HOST' is required".to_string()))?
        .clone();

    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(connection_timeout);

    if let Some(ca_pem) = credentials.get(KEY_CA_CERT) {
        let cert = reqwest::tls::Certificate::from_pem(ca_pem.as_bytes())
            .map_err(|e| ConnectorError::ConnectionError(format!("Invalid CA certificate: {e}")))?;
        builder = builder.add_root_certificate(cert);
    }

    let http = builder
        .build()
        .map_err(|e| ConnectorError::ConnectionError(format!("Failed to build HTTP client: {e}")))?;

    let auth = if let Some(encoded) = credentials.get(KEY_API_KEY) {
        EsAuth::ApiKey {
            encoded: encoded.clone(),
        }
    } else if let (Some(username), Some(password)) = (credentials.get(KEY_USERNAME), credentials.get(KEY_PASSWORD)) {
        EsAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }
    } else {
        EsAuth::None
    };

    Ok(EsClient { http, base_url, auth })
}

#[async_trait::async_trait]
impl FlightConnector for ElasticsearchConnector {
    fn provider(&self) -> String {
        Provider::Elasticsearch.as_str().to_string()
    }

    fn description(&self) -> String {
        "Elasticsearch connector".to_string()
    }

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
    ) -> Result<Arc<dyn TabularReader>, ConnectorError> {
        let credentials = extract_credentials(data_connection)?;
        let cache_key = data_connection.metadata.id.clone();
        let connection_timeout = self.config.connection_timeout();
        let client = self
            .clients
            .try_get_with(cache_key, async { build_client(&credentials, connection_timeout) })
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to get Elasticsearch client: {e}")))?;

        let default_index = data_connection.resource.properties.get("index").cloned();

        Ok(Arc::new(ElasticsearchReader { client, default_index }))
    }
}

pub struct ElasticsearchReader {
    client: EsClient,
    default_index: Option<String>,
}

#[async_trait::async_trait]
impl TabularReader for ElasticsearchReader {
    fn provider(&self) -> String {
        Provider::Elasticsearch.as_str().to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<TabularState>, ConnectorError> {
        let request = EsRequestInput::parse(query)?;
        let index = request.resolve_index(self.default_index.as_deref())?;

        let response = self
            .client
            .get(&format!("/{}/_mapping", index))
            .send()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to get mapping: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ConnectorError::ConnectionError(format!(
                "Mapping request failed (HTTP {status}): {body}"
            )));
        }

        let mapping_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to parse mapping response: {e}")))?;

        let mapping_fields = types::parse_mapping(&mapping_json);

        if mapping_fields.is_empty() {
            return Err(ConnectorError::NoDataError);
        }

        let schema = types::mapping_fields_to_schema(&mapping_fields);
        Ok(Arc::new(TabularState::new(query.to_owned(), Arc::new(schema))))
    }

    async fn read(&self, state: Arc<TabularState>, options: &QueryOptions) -> QueryOutput {
        let request = EsRequestInput::parse(&state.query)?;
        let index = request.resolve_index(self.default_index.as_deref())?;
        let schema = state.schema.clone();
        let batch_size = options.batch_size as u64;
        let total_limit = request.size;
        let client = self.client.clone();

        let stream = async_stream::try_stream! {
            let pit_response: serde_json::Value = client
                .post(&format!("/{}/_pit?keep_alive=5m", index))
                .send()
                .await
                .map_err(|e| ConnectorError::ConnectionError(format!("Failed to open PIT: {e}")))?
                .json()
                .await
                .map_err(|e| ConnectorError::ConnectionError(format!("Failed to parse PIT response: {e}")))?;

            let pit_id = pit_response
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ConnectorError::ConnectionError("PIT response missing 'id'".to_string()))?
                .to_string();

            let mut search_after: Option<serde_json::Value> = None;
            let mut pit_id_current = pit_id;
            let mut total_fetched: u64 = 0;
            let mut loop_error: Option<ConnectorError> = None;

            loop {
                let remaining = total_limit.map(|limit| limit.saturating_sub(total_fetched));
                if remaining == Some(0) {
                    break;
                }

                let page_size = match remaining {
                    Some(r) => r.min(batch_size),
                    None => batch_size,
                };

                let body = request.build_pit_search_body(&pit_id_current, page_size, search_after.as_ref());

                let response_json = match fetch_search_page(&client, body).await {
                    Ok(v) => v,
                    Err(e) => { loop_error = Some(e); break; }
                };

                if let Some(new_pit_id) = response_json.get("pit_id").and_then(|v| v.as_str()) {
                    pit_id_current = new_pit_id.to_string();
                }

                let hits = response_json
                    .get("hits")
                    .and_then(|h| h.get("hits"))
                    .and_then(|h| h.as_array());

                let Some(hits) = hits else { break };

                if hits.is_empty() {
                    break;
                }

                let num_hits = hits.len() as u64;
                match hits_to_record_batch(&schema, hits) {
                    Ok(batch) => yield batch,
                    Err(e) => { loop_error = Some(e); break; }
                }

                total_fetched += num_hits;

                search_after = hits.last().and_then(|h| h.get("sort").cloned());
                if search_after.is_none() {
                    break;
                }

                if num_hits < page_size {
                    break;
                }
            }

            close_pit(&client, &pit_id_current).await;

            if let Some(e) = loop_error {
                Err(e)?;
            }
        };

        Ok(Box::pin(stream))
    }

    async fn test_connection(&self) -> Result<(), ConnectorError> {
        let response = self
            .client
            .get("/")
            .send()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Connection test failed: {e}")))?;

        if !response.status().is_success() {
            return Err(ConnectorError::ConnectionError(format!(
                "Elasticsearch returned HTTP {}",
                response.status()
            )));
        }

        Ok(())
    }
}

async fn fetch_search_page(client: &EsClient, body: serde_json::Value) -> Result<serde_json::Value, ConnectorError> {
    let response = client
        .post("/_search")
        .json(&body)
        .send()
        .await
        .map_err(|e| ConnectorError::ConnectionError(format!("Search failed: {e}")))?;
    let status = response.status();
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| ConnectorError::ConnectionError(format!("Failed to parse search response: {e}")))?;
    if !status.is_success() {
        let reason = json
            .get("error")
            .and_then(|e| e.get("reason"))
            .and_then(|r| r.as_str())
            .unwrap_or("unknown error");
        return Err(ConnectorError::SQLError(format!(
            "Search failed (HTTP {status}): {reason}"
        )));
    }
    Ok(json)
}

async fn close_pit(client: &EsClient, pit_id: &str) {
    if let Err(e) = client
        .delete("/_pit")
        .json(&serde_json::json!({ "id": pit_id }))
        .send()
        .await
    {
        tracing::error!("Failed to close PIT: {e}");
    }
}

fn hits_to_record_batch(schema: &Arc<Schema>, hits: &[serde_json::Value]) -> Result<RecordBatch, ConnectorError> {
    let arrays: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|field| extract_column_from_hits(field.name(), field.data_type(), hits))
        .collect::<Result<_, _>>()?;

    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(|e| ConnectorError::SQLError(e.to_string()))
}

fn extract_column_from_hits(
    field_path: &str,
    data_type: &ArrowDataType,
    hits: &[serde_json::Value],
) -> Result<ArrayRef, ConnectorError> {
    let values: Vec<Option<&serde_json::Value>> = hits
        .iter()
        .map(|hit| {
            let source = hit.get("_source")?;
            resolve_path(source, field_path)
        })
        .collect();

    json_values_to_array(data_type, &values)
}

fn resolve_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
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
        let connector = ElasticsearchConnector::new(
            Duration::from_secs(300),
            Duration::from_secs(60),
            100,
            ConnectorConfig::default(),
        );
        assert_eq!(connector.provider(), "elasticsearch");
    }

    #[test]
    fn test_extract_credentials_success() {
        let conn = DataConnectionResource {
            metadata: commons::api::ResourceMetadata {
                id: "conn-1".to_string(),
                tenant_id: Some("t-1".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: commons::api::connections::DataConnection {
                name: "test-es".to_string(),
                data_connection_type_id: "es-type".to_string(),
                format: commons::api::connections::DataFormat::Tabular,
                admin: Some(Admin::Secret {
                    name: "test-es".to_string(),
                    secret: Arc::new(HashMap::from([(
                        KEY_HOST.to_string(),
                        "http://localhost:9200".to_string(),
                    )])),
                }),
                properties: HashMap::new(),
            },
            status: Default::default(),
        };
        let result = extract_credentials(&conn);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get(KEY_HOST).unwrap(), "http://localhost:9200");
    }

    #[test]
    fn test_extract_credentials_missing() {
        let conn = DataConnectionResource {
            metadata: commons::api::ResourceMetadata {
                id: "conn-1".to_string(),
                tenant_id: Some("t-1".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: commons::api::connections::DataConnection {
                name: "test-es".to_string(),
                data_connection_type_id: "es-type".to_string(),
                format: commons::api::connections::DataFormat::Tabular,
                admin: None,
                properties: HashMap::new(),
            },
            status: Default::default(),
        };
        assert!(extract_credentials(&conn).is_err());
    }

    #[test]
    fn test_resolve_path_simple() {
        let val = serde_json::json!({"name": "Alice"});
        assert_eq!(resolve_path(&val, "name").unwrap(), "Alice");
    }

    #[test]
    fn test_resolve_path_nested() {
        let val = serde_json::json!({"user": {"name": "Alice", "age": 30}});
        assert_eq!(resolve_path(&val, "user.name").unwrap(), "Alice");
        assert_eq!(resolve_path(&val, "user.age").unwrap(), 30);
    }

    #[test]
    fn test_resolve_path_missing() {
        let val = serde_json::json!({"name": "Alice"});
        assert!(resolve_path(&val, "missing").is_none());
        assert!(resolve_path(&val, "a.b.c").is_none());
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
    fn test_hits_to_record_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("title", ArrowDataType::Utf8, true),
            Field::new("count", ArrowDataType::Int64, true),
        ]));
        let hits = vec![
            serde_json::json!({"_source": {"title": "hello", "count": 10}}),
            serde_json::json!({"_source": {"title": "world", "count": 20}}),
        ];
        let batch = hits_to_record_batch(&schema, &hits).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);

        let title_arr = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(title_arr.value(0), "hello");
        assert_eq!(title_arr.value(1), "world");

        let count_arr = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(count_arr.value(0), 10);
        assert_eq!(count_arr.value(1), 20);
    }

    #[test]
    fn test_hits_to_record_batch_nested() {
        let schema = Arc::new(Schema::new(vec![Field::new("user.name", ArrowDataType::Utf8, true)]));
        let hits = vec![
            serde_json::json!({"_source": {"user": {"name": "Alice"}}}),
            serde_json::json!({"_source": {"user": {"name": "Bob"}}}),
        ];
        let batch = hits_to_record_batch(&schema, &hits).unwrap();
        let name_arr = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(name_arr.value(0), "Alice");
        assert_eq!(name_arr.value(1), "Bob");
    }

    #[test]
    fn test_hits_to_record_batch_null_fields() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("title", ArrowDataType::Utf8, true),
            Field::new("missing", ArrowDataType::Utf8, true),
        ]));
        let hits = vec![serde_json::json!({"_source": {"title": "hello"}})];
        let batch = hits_to_record_batch(&schema, &hits).unwrap();
        assert_eq!(batch.num_rows(), 1);

        let missing_arr = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert!(missing_arr.is_null(0));
    }
}
