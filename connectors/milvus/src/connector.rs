use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array, StringArray,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use commons::api::connections::DataConnectionResource;
use commons::api::connector::CredentialsResolver;
use commons::api::connector::{DataReader, FlightConnector, Query, QueryOptions, QueryOutput};
use commons::api::errors::ConnectorError;
use commons::utils::config::ConnectorConfig;
use moka::future::Cache;

use crate::query::{MilvusOperation, MilvusRequestInput};

const KEY_URI: &str = "MILVUS_URI";
const KEY_TOKEN: &str = "MILVUS_TOKEN";
const KEY_DATABASE: &str = "MILVUS_DATABASE";
const KEY_CA_CERT: &str = "MILVUS_CA_CERT";

#[derive(Clone)]
struct MilvusClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
    database: Option<String>,
}

impl MilvusClient {
    async fn execute(&self, path: &str, mut body: serde_json::Value) -> Result<serde_json::Value, ConnectorError> {
        if let (Some(db), Some(obj)) = (&self.database, body.as_object_mut()) {
            obj.entry("dbName").or_insert(serde_json::json!(db));
        }

        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = self.http.post(&url);
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }

        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Milvus request failed: {e}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to read Milvus response body: {e}")))?;

        if !status.is_success() {
            return Err(ConnectorError::ConnectionError(format!(
                "Milvus error (HTTP {status}): {body}"
            )));
        }

        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to parse Milvus response: {e}")))?;

        let code = json.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        if code != 0 {
            let message = json.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
            return Err(ConnectorError::ConnectionError(format!(
                "Milvus error (code {code}): {message}"
            )));
        }

        Ok(json)
    }
}

pub struct MilvusConnector {
    clients: Cache<String, MilvusClient>,
    config: ConnectorConfig,
}

impl MilvusConnector {
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

const PROVIDER: &str = "milvus";

#[async_trait::async_trait]
impl FlightConnector for MilvusConnector {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    fn description(&self) -> String {
        "Milvus vector database connector".to_string()
    }

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

                let base_url = credentials
                    .get(KEY_URI)
                    .ok_or_else(|| ConnectorError::ConnectionError("MILVUS_URI is required".to_string()))?
                    .clone();
                let token = credentials.get(KEY_TOKEN).cloned();
                let database = credentials.get(KEY_DATABASE).cloned();

                let connection_timeout = self.config.connection_timeout();
                let request_timeout = Duration::from_secs(connection_timeout.as_secs().max(10) * 3);

                let mut builder = reqwest::Client::builder()
                    .connect_timeout(connection_timeout)
                    .timeout(request_timeout);

                if let Some(ca_pem) = credentials.get(KEY_CA_CERT) {
                    let cert = reqwest::tls::Certificate::from_pem(ca_pem.as_bytes())
                        .map_err(|e| ConnectorError::ConnectionError(format!("Invalid CA certificate: {e}")))?;
                    builder = builder.add_root_certificate(cert);
                }

                let http = builder
                    .build()
                    .map_err(|e| ConnectorError::ConnectionError(format!("Failed to build HTTP client: {e}")))?;

                Ok::<_, ConnectorError>(MilvusClient {
                    http,
                    base_url,
                    token,
                    database,
                })
            })
            .await
            .map_err(|e| Arc::try_unwrap(e).unwrap_or_else(|arc| (*arc).clone()))?;

        Ok(Arc::new(MilvusReader { client }))
    }
}

pub struct MilvusReader {
    client: MilvusClient,
}

struct MilvusFieldDesc {
    name: String,
    field_type: String,
}

#[async_trait::async_trait]
impl DataReader for MilvusReader {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<Query>, ConnectorError> {
        let request = MilvusRequestInput::parse(query)?;
        let field_descs = self.describe_collection(&request.collection_name).await?;
        let output_fields = request.output_fields();
        let schema = build_schema(&field_descs, output_fields.as_deref(), &request.operation);
        Ok(Arc::new(Query::new(query.to_owned(), Arc::new(schema))))
    }

    async fn read_tabular(&self, query: Arc<Query>, options: &QueryOptions) -> QueryOutput {
        let request = MilvusRequestInput::parse(&query.query)?;
        let schema = query.schema.clone();
        let batch_size = options.batch_size;

        match request.operation {
            MilvusOperation::Query => self.read_query_paginated(request, schema, batch_size),
            MilvusOperation::Search | MilvusOperation::Get => {
                let endpoint = operation_endpoint(&request.operation);
                let response = self.client.execute(endpoint, request.body).await?;
                let rows = extract_data_rows(response);

                let stream = async_stream::try_stream! {
                    let mut offset = 0;
                    while offset < rows.len() {
                        let end = (offset + batch_size).min(rows.len());
                        let batch = rows_to_record_batch(&schema, &rows[offset..end])?;
                        yield batch;
                        offset = end;
                    }
                };
                Ok(Box::pin(stream))
            },
        }
    }

    async fn check_connection(&self) -> Result<(), ConnectorError> {
        self.client
            .execute("/v2/vectordb/collections/list", serde_json::json!({}))
            .await?;
        Ok(())
    }
}

impl MilvusReader {
    async fn describe_collection(&self, collection_name: &str) -> Result<Vec<MilvusFieldDesc>, ConnectorError> {
        let body = serde_json::json!({ "collectionName": collection_name });
        let response = self.client.execute("/v2/vectordb/collections/describe", body).await?;

        let fields = response
            .get("data")
            .and_then(|d| d.get("fields"))
            .and_then(|f| f.as_array())
            .ok_or_else(|| ConnectorError::ConnectionError("Invalid describe response: missing fields".to_string()))?;

        Ok(fields
            .iter()
            .filter_map(|f| {
                let name = f.get("name")?.as_str()?.to_string();
                let field_type = f.get("type")?.as_str()?.to_string();
                Some(MilvusFieldDesc { name, field_type })
            })
            .collect())
    }

    fn read_query_paginated(&self, request: MilvusRequestInput, schema: Arc<Schema>, batch_size: usize) -> QueryOutput {
        let client = self.client.clone();
        let page_size = batch_size as i64;

        let stream = async_stream::try_stream! {
            let client_offset = request.offset.unwrap_or(0);
            let client_limit = request.limit;
            let mut fetched: i64 = 0;

            loop {
                let remaining = client_limit.map(|l| l - fetched);
                let this_page = match remaining {
                    Some(r) if r <= 0 => break,
                    Some(r) => r.min(page_size),
                    None => page_size,
                };

                let mut body = request.body.clone();
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("limit".to_string(), serde_json::json!(this_page));
                    obj.insert("offset".to_string(), serde_json::json!(client_offset + fetched));
                }

                let response = client.execute("/v2/vectordb/entities/query", body).await?;
                let rows = extract_data_rows(response);

                if rows.is_empty() {
                    break;
                }

                let num_rows = rows.len();
                let batch = rows_to_record_batch(&schema, &rows)?;
                yield batch;

                fetched += num_rows as i64;

                if (num_rows as i64) < this_page {
                    break;
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

fn operation_endpoint(op: &MilvusOperation) -> &'static str {
    match op {
        MilvusOperation::Query => "/v2/vectordb/entities/query",
        MilvusOperation::Search => "/v2/vectordb/entities/search",
        MilvusOperation::Get => "/v2/vectordb/entities/get",
    }
}

fn extract_data_rows(response: serde_json::Value) -> Vec<serde_json::Value> {
    match response {
        serde_json::Value::Object(mut map) => match map.remove("data") {
            Some(serde_json::Value::Array(arr)) => arr,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn build_schema(
    field_descs: &[MilvusFieldDesc],
    output_fields: Option<&[String]>,
    operation: &MilvusOperation,
) -> Schema {
    let type_map: HashMap<&str, &str> = field_descs
        .iter()
        .map(|f| (f.name.as_str(), f.field_type.as_str()))
        .collect();

    let mut arrow_fields: Vec<Field> = if let Some(fields) = output_fields {
        let expanded_fields: Vec<&str> = fields
            .iter()
            .flat_map(|name| {
                if name == "*" {
                    field_descs.iter().map(|field| field.name.as_str()).collect::<Vec<_>>()
                } else {
                    vec![name.as_str()]
                }
            })
            .collect();

        expanded_fields
            .into_iter()
            .map(|name| {
                let arrow_type = type_map
                    .get(name)
                    .map(|t| milvus_type_to_arrow(t))
                    .unwrap_or(ArrowDataType::Utf8);
                Field::new(name, arrow_type, true)
            })
            .collect()
    } else {
        field_descs
            .iter()
            .filter(|f| !is_vector_type(&f.field_type))
            .map(|f| Field::new(&f.name, milvus_type_to_arrow(&f.field_type), true))
            .collect()
    };

    if matches!(operation, MilvusOperation::Search) && !arrow_fields.iter().any(|f| f.name() == "distance") {
        arrow_fields.push(Field::new("distance", ArrowDataType::Float64, true));
    }

    Schema::new(arrow_fields)
}

fn milvus_type_to_arrow(milvus_type: &str) -> ArrowDataType {
    match milvus_type {
        "Bool" => ArrowDataType::Boolean,
        "Int8" => ArrowDataType::Int8,
        "Int16" => ArrowDataType::Int16,
        "Int32" => ArrowDataType::Int32,
        "Int64" => ArrowDataType::Int64,
        "Float" => ArrowDataType::Float32,
        "Double" => ArrowDataType::Float64,
        _ => ArrowDataType::Utf8,
    }
}

fn is_vector_type(milvus_type: &str) -> bool {
    matches!(
        milvus_type,
        "FloatVector" | "Float16Vector" | "BFloat16Vector" | "BinaryVector" | "SparseFloatVector"
    )
}

fn rows_to_record_batch(schema: &Arc<Schema>, rows: &[serde_json::Value]) -> Result<RecordBatch, ConnectorError> {
    let arrays: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|field| {
            let values: Vec<Option<&serde_json::Value>> = rows.iter().map(|row| row.get(field.name())).collect();
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

    #[test]
    fn test_milvus_connector_provider() {
        let connector = MilvusConnector::new(
            Duration::from_secs(300),
            Duration::from_secs(60),
            100,
            ConnectorConfig::default(),
        );
        assert_eq!(connector.provider(), "milvus");
    }

    #[test]
    fn test_milvus_type_to_arrow() {
        assert_eq!(milvus_type_to_arrow("Bool"), ArrowDataType::Boolean);
        assert_eq!(milvus_type_to_arrow("Int8"), ArrowDataType::Int8);
        assert_eq!(milvus_type_to_arrow("Int16"), ArrowDataType::Int16);
        assert_eq!(milvus_type_to_arrow("Int32"), ArrowDataType::Int32);
        assert_eq!(milvus_type_to_arrow("Int64"), ArrowDataType::Int64);
        assert_eq!(milvus_type_to_arrow("Float"), ArrowDataType::Float32);
        assert_eq!(milvus_type_to_arrow("Double"), ArrowDataType::Float64);
        assert_eq!(milvus_type_to_arrow("VarChar"), ArrowDataType::Utf8);
        assert_eq!(milvus_type_to_arrow("JSON"), ArrowDataType::Utf8);
        assert_eq!(milvus_type_to_arrow("Unknown"), ArrowDataType::Utf8);
    }

    #[test]
    fn test_is_vector_type() {
        assert!(is_vector_type("FloatVector"));
        assert!(is_vector_type("Float16Vector"));
        assert!(is_vector_type("BFloat16Vector"));
        assert!(is_vector_type("BinaryVector"));
        assert!(is_vector_type("SparseFloatVector"));
        assert!(!is_vector_type("Int64"));
        assert!(!is_vector_type("VarChar"));
        assert!(!is_vector_type("Bool"));
    }

    #[test]
    fn test_build_schema_excludes_vectors() {
        let field_descs = vec![
            MilvusFieldDesc {
                name: "id".to_string(),
                field_type: "Int64".to_string(),
            },
            MilvusFieldDesc {
                name: "name".to_string(),
                field_type: "VarChar".to_string(),
            },
            MilvusFieldDesc {
                name: "vector".to_string(),
                field_type: "FloatVector".to_string(),
            },
            MilvusFieldDesc {
                name: "active".to_string(),
                field_type: "Bool".to_string(),
            },
        ];
        let schema = build_schema(&field_descs, None, &MilvusOperation::Query);
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(*schema.field(0).data_type(), ArrowDataType::Int64);
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(*schema.field(1).data_type(), ArrowDataType::Utf8);
        assert_eq!(schema.field(2).name(), "active");
        assert_eq!(*schema.field(2).data_type(), ArrowDataType::Boolean);
    }

    #[test]
    fn test_build_schema_with_output_fields() {
        let field_descs = vec![
            MilvusFieldDesc {
                name: "id".to_string(),
                field_type: "Int64".to_string(),
            },
            MilvusFieldDesc {
                name: "name".to_string(),
                field_type: "VarChar".to_string(),
            },
            MilvusFieldDesc {
                name: "vector".to_string(),
                field_type: "FloatVector".to_string(),
            },
        ];
        let output = vec!["id".to_string(), "name".to_string()];
        let schema = build_schema(&field_descs, Some(&output), &MilvusOperation::Query);
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "name");
    }

    #[test]
    fn test_build_schema_expands_wildcard_output_fields() {
        let field_descs = vec![
            MilvusFieldDesc {
                name: "id".to_string(),
                field_type: "Int64".to_string(),
            },
            MilvusFieldDesc {
                name: "name".to_string(),
                field_type: "VarChar".to_string(),
            },
        ];
        let output = vec!["*".to_string()];
        let schema = build_schema(&field_descs, Some(&output), &MilvusOperation::Query);

        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(*schema.field(0).data_type(), ArrowDataType::Int64);
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(*schema.field(1).data_type(), ArrowDataType::Utf8);
    }

    #[test]
    fn test_build_schema_search_adds_distance() {
        let field_descs = vec![
            MilvusFieldDesc {
                name: "id".to_string(),
                field_type: "Int64".to_string(),
            },
            MilvusFieldDesc {
                name: "vector".to_string(),
                field_type: "FloatVector".to_string(),
            },
        ];
        let schema = build_schema(&field_descs, None, &MilvusOperation::Search);
        assert!(schema.fields().iter().any(|f| f.name() == "distance"));
        let distance = schema.fields().iter().find(|f| f.name() == "distance").unwrap();
        assert_eq!(*distance.data_type(), ArrowDataType::Float64);
    }

    #[test]
    fn test_build_schema_search_no_duplicate_distance() {
        let field_descs = vec![MilvusFieldDesc {
            name: "id".to_string(),
            field_type: "Int64".to_string(),
        }];
        let output = vec!["id".to_string(), "distance".to_string()];
        let schema = build_schema(&field_descs, Some(&output), &MilvusOperation::Search);
        let distance_count = schema.fields().iter().filter(|f| f.name() == "distance").count();
        assert_eq!(distance_count, 1);
    }

    #[test]
    fn test_extract_data_rows() {
        let response = serde_json::json!({
            "code": 0,
            "data": [
                {"id": 1, "name": "Alice"},
                {"id": 2, "name": "Bob"}
            ]
        });
        let rows = extract_data_rows(response);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_extract_data_rows_empty() {
        let response = serde_json::json!({"code": 0, "data": []});
        let rows = extract_data_rows(response);
        assert!(rows.is_empty());
    }

    #[test]
    fn test_extract_data_rows_missing() {
        let response = serde_json::json!({"code": 0});
        let rows = extract_data_rows(response);
        assert!(rows.is_empty());
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
    fn test_rows_to_record_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", ArrowDataType::Int64, true),
            Field::new("name", ArrowDataType::Utf8, true),
        ]));
        let rows = vec![
            serde_json::json!({"id": 1, "name": "Alice"}),
            serde_json::json!({"id": 2, "name": "Bob"}),
        ];
        let batch = rows_to_record_batch(&schema, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);

        let id_arr = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(id_arr.value(0), 1);
        assert_eq!(id_arr.value(1), 2);

        let name_arr = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(name_arr.value(0), "Alice");
        assert_eq!(name_arr.value(1), "Bob");
    }

    #[test]
    fn test_rows_to_record_batch_missing_fields() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", ArrowDataType::Int64, true),
            Field::new("missing", ArrowDataType::Utf8, true),
        ]));
        let rows = vec![serde_json::json!({"id": 1})];
        let batch = rows_to_record_batch(&schema, &rows).unwrap();
        assert_eq!(batch.num_rows(), 1);

        let missing_arr = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert!(missing_arr.is_null(0));
    }

    #[test]
    fn test_operation_endpoint() {
        assert_eq!(
            operation_endpoint(&MilvusOperation::Query),
            "/v2/vectordb/entities/query"
        );
        assert_eq!(
            operation_endpoint(&MilvusOperation::Search),
            "/v2/vectordb/entities/search"
        );
        assert_eq!(operation_endpoint(&MilvusOperation::Get), "/v2/vectordb/entities/get");
    }
}
