use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array, StringArray,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::errors::ConnectorError;
use commons::api::tabular::{FlightConnector, QueryOptions, QueryOutput, TabularReader, TabularState};
use commons::utils::config::ConnectorConfig;
use milvus::v2::prelude::{
    ClientV2, ConnectConfig, FieldData, GetRequest, Ids, QueryRequest, QueryResponse, SearchRequest, SearchResponse,
    SearchVectors,
};
use moka::future::Cache;

use crate::query::{MilvusOperation, MilvusRequestInput};

const KEY_HOST: &str = "MILVUS_HOST";
const KEY_PORT: &str = "MILVUS_PORT";
const KEY_TOKEN: &str = "MILVUS_TOKEN";
const KEY_DATABASE: &str = "MILVUS_DATABASE";
const DEFAULT_PORT: &str = "19530";

pub struct MilvusConnector {
    clients: Cache<String, ClientV2>,
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

fn extract_credentials(
    data_connection: &DataConnectionResource,
) -> Result<Arc<HashMap<String, String>>, ConnectorError> {
    match &data_connection.resource.admin {
        Some(Admin::Secret { name: _, secret }) => Ok(secret.clone()),
        _ => Err(ConnectorError::ConnectionError(
            "Milvus credentials are required".to_string(),
        )),
    }
}

fn map_milvus_error(e: milvus::v2::error::Error) -> ConnectorError {
    ConnectorError::ConnectionError(format!("Milvus error: {e}"))
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
        enable_cache: bool,
        data_connection: &DataConnectionResource,
    ) -> Result<Arc<dyn TabularReader>, ConnectorError> {
        let credentials = extract_credentials(data_connection)?;

        let host = credentials
            .get(KEY_HOST)
            .ok_or_else(|| ConnectorError::ConnectionError("MILVUS_HOST is required".to_string()))?
            .clone();

        let port = credentials.get(KEY_PORT).map(|s| s.as_str()).unwrap_or(DEFAULT_PORT);

        let uri = format!("http://{host}:{port}");
        let token = credentials.get(KEY_TOKEN).cloned();
        let database = credentials.get(KEY_DATABASE).cloned();

        let connection_timeout = self.config.connection_timeout();
        let mut config = ConnectConfig::new().uri(&uri).connect_timeout(connection_timeout);
        if let Some(ref token) = token {
            config = config.token(token);
        }
        if let Some(ref db) = database {
            config = config.database(db);
        }

        if !enable_cache {
            return Ok(Arc::new(MilvusReader {
                client: ClientV2::new(&config).await.map_err(map_milvus_error)?,
            }));
        }

        let cache_key = data_connection.metadata.id.clone();

        let client = self
            .clients
            .try_get_with(cache_key, async {
                ClientV2::new(&config).await.map_err(map_milvus_error)
            })
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to get Milvus client: {e}")))?;

        Ok(Arc::new(MilvusReader { client }))
    }
}

pub struct MilvusReader {
    client: ClientV2,
}

#[async_trait::async_trait]
impl TabularReader for MilvusReader {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<TabularState>, ConnectorError> {
        let mut request = MilvusRequestInput::parse(query)?;

        if let MilvusOperation::Query = request.operation() {
            request.limit = Some(1);
        }

        let field_data = match request.operation() {
            MilvusOperation::Query => self.execute_query(&request).await?,
            MilvusOperation::Search => self.execute_search(&request).await?,
            MilvusOperation::Get => self.execute_get(&request).await?,
        };

        let field_data = normalize_field_order(&request, field_data);
        let schema = schema_from_field_data(&field_data);
        Ok(Arc::new(TabularState::new(query.to_owned(), Arc::new(schema))))
    }

    async fn read(&self, state: Arc<TabularState>, options: &QueryOptions) -> QueryOutput {
        let request = MilvusRequestInput::parse(&state.query)?;
        let batch_size = options.batch_size;
        let schema = state.schema.clone();

        match request.operation() {
            MilvusOperation::Query => self.read_query_paginated(request, schema, batch_size),
            MilvusOperation::Search | MilvusOperation::Get => {
                let field_data = match request.operation() {
                    MilvusOperation::Search => self.execute_search(&request).await?,
                    MilvusOperation::Get => self.execute_get(&request).await?,
                    _ => unreachable!(),
                };
                let field_data = reorder_fields(&schema, field_data);
                let total_rows = field_data.first().map(field_data_len).unwrap_or(0);

                let stream = async_stream::try_stream! {
                    let mut offset = 0;
                    while offset < total_rows {
                        let end = (offset + batch_size).min(total_rows);
                        let batch = fields_to_batch(&schema, &field_data, offset, end)?;
                        yield batch;
                        offset = end;
                    }
                };
                Ok(Box::pin(stream))
            },
        }
    }

    async fn check_connection(&self) -> Result<(), ConnectorError> {
        use milvus::v2::request::utility::CheckHealthRequest;
        let req = CheckHealthRequest::builder()
            .build()
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to build health check request: {e}")))?;
        let resp = self
            .client
            .check_health(req)
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Milvus health check failed: {e}")))?;
        if !resp.is_healthy() {
            return Err(ConnectorError::ConnectionError("Milvus reported unhealthy".to_string()));
        }
        Ok(())
    }
}

impl MilvusReader {
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

                let mut builder = QueryRequest::builder()
                    .collection_name(&request.collection_name)
                    .filter(request.filter.as_deref().unwrap_or(""))
                    .offset(client_offset + fetched)
                    .limit(this_page);

                if let Some(ref fields) = request.output_fields {
                    builder = builder.output_fields(fields.iter().map(|s| s.as_str()));
                }

                let req = builder
                    .build()
                    .map_err(|e| ConnectorError::InvalidRequest(format!("Failed to build query request: {e}")))?;

                let response: QueryResponse = client.query(req).await.map_err(map_milvus_error)?;
                let field_data = response.results().get_output_fields().to_vec();
                let field_data = reorder_fields(&schema, field_data);
                let rows = field_data.first().map(field_data_len).unwrap_or(0);

                if rows == 0 {
                    break;
                }

                let batch = fields_to_batch(&schema, &field_data, 0, rows)?;
                yield batch;

                fetched += rows as i64;

                if (rows as i64) < this_page {
                    break;
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn execute_query(&self, request: &MilvusRequestInput) -> Result<Vec<FieldData>, ConnectorError> {
        let mut builder = QueryRequest::builder()
            .collection_name(&request.collection_name)
            .filter(request.filter.as_deref().unwrap_or(""));

        if let Some(ref fields) = request.output_fields {
            builder = builder.output_fields(fields.iter().map(|s| s.as_str()));
        }
        if let Some(limit) = request.limit {
            builder = builder.limit(limit);
        }
        if let Some(offset) = request.offset {
            builder = builder.offset(offset);
        }

        let req = builder
            .build()
            .map_err(|e| ConnectorError::InvalidRequest(format!("Failed to build query request: {e}")))?;

        let response: QueryResponse = self.client.query(req).await.map_err(map_milvus_error)?;

        Ok(response.results().get_output_fields().to_vec())
    }

    async fn execute_search(&self, request: &MilvusRequestInput) -> Result<Vec<FieldData>, ConnectorError> {
        let vectors = request
            .data
            .as_ref()
            .ok_or_else(|| ConnectorError::InvalidRequest("Search requires 'data' field".to_string()))?;

        let anns_field = request
            .anns_field
            .as_deref()
            .ok_or_else(|| ConnectorError::InvalidRequest("Search requires 'annsField' field".to_string()))?;

        let mut builder = SearchRequest::builder()
            .collection_name(&request.collection_name)
            .vectors(SearchVectors::Float(vectors.clone()))
            .vector_field(anns_field);

        if let Some(ref filter) = request.filter {
            builder = builder.filter(filter);
        }
        if let Some(limit) = request.limit {
            builder = builder.limit(limit);
        }
        if let Some(ref fields) = request.output_fields {
            builder = builder.output_fields(fields.iter().map(|s| s.as_str()));
        }

        let req = builder
            .build()
            .map_err(|e| ConnectorError::InvalidRequest(format!("Failed to build search request: {e}")))?;

        let response: SearchResponse = self.client.search(req).await.map_err(map_milvus_error)?;

        // TODO: only the first query vector's results are returned; multi-vector queries are not yet supported.
        Ok(response
            .results()
            .get_results()
            .first()
            .map(|r| r.get_output_fields().to_vec())
            .unwrap_or_default())
    }

    async fn execute_get(&self, request: &MilvusRequestInput) -> Result<Vec<FieldData>, ConnectorError> {
        let id_value = request
            .id
            .as_ref()
            .ok_or_else(|| ConnectorError::InvalidRequest("Get requires 'id' field".to_string()))?;

        let ids = parse_ids(id_value)?;

        let mut builder = GetRequest::builder().collection_name(&request.collection_name).ids(ids);

        if let Some(ref fields) = request.output_fields {
            builder = builder.output_fields(fields.iter().map(|s| s.as_str()));
        }

        let req = builder
            .build()
            .map_err(|e| ConnectorError::InvalidRequest(format!("Failed to build get request: {e}")))?;

        let response: QueryResponse = self.client.get(req).await.map_err(map_milvus_error)?;

        Ok(response.results().get_output_fields().to_vec())
    }
}

fn parse_ids(value: &serde_json::Value) -> Result<Ids, ConnectorError> {
    match value {
        serde_json::Value::Number(n) => {
            let id = n
                .as_i64()
                .ok_or_else(|| ConnectorError::InvalidRequest("Invalid ID number".to_string()))?;
            Ok(Ids::Int64(vec![id]))
        },
        serde_json::Value::String(s) => Ok(Ids::VarChar(vec![s.clone()])),
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return Err(ConnectorError::InvalidRequest("Empty ID array".to_string()));
            }
            if arr[0].is_number() {
                let ids: Vec<i64> = arr
                    .iter()
                    .map(|v| {
                        v.as_i64()
                            .ok_or_else(|| ConnectorError::InvalidRequest("Invalid ID number".to_string()))
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Ids::Int64(ids))
            } else {
                let ids: Vec<String> = arr
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(|s| s.to_string())
                            .ok_or_else(|| ConnectorError::InvalidRequest("Invalid string ID".to_string()))
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Ids::VarChar(ids))
            }
        },
        _ => Err(ConnectorError::InvalidRequest("Invalid ID format".to_string())),
    }
}

/// Ensure deterministic field ordering: use `output_fields` order if specified, otherwise sort by name.
/// Milvus may return fields in arbitrary order across calls; this prevents schema mismatches
/// between `get_flight_info` and `do_get`.
fn normalize_field_order(request: &MilvusRequestInput, mut fields: Vec<FieldData>) -> Vec<FieldData> {
    if let Some(ref output_fields) = request.output_fields {
        let mut ordered = Vec::with_capacity(output_fields.len());
        for name in output_fields {
            if let Some(pos) = fields.iter().position(|f| f.name() == name) {
                ordered.push(fields.swap_remove(pos));
            }
        }
        ordered.extend(fields);
        ordered
    } else {
        fields.sort_by(|a, b| a.name().cmp(b.name()));
        fields
    }
}

fn reorder_fields(schema: &Schema, mut fields: Vec<FieldData>) -> Vec<FieldData> {
    let mut ordered = Vec::with_capacity(schema.fields().len());
    for schema_field in schema.fields() {
        if let Some(pos) = fields.iter().position(|f| f.name() == schema_field.name()) {
            ordered.push(fields.swap_remove(pos));
        }
    }
    ordered
}

fn field_data_arrow_type(field: &FieldData) -> ArrowDataType {
    match field {
        FieldData::Bool { .. } => ArrowDataType::Boolean,
        FieldData::Int8 { .. } => ArrowDataType::Int8,
        FieldData::Int16 { .. } => ArrowDataType::Int16,
        FieldData::Int32 { .. } => ArrowDataType::Int32,
        FieldData::Int64 { .. } => ArrowDataType::Int64,
        FieldData::Float { .. } => ArrowDataType::Float32,
        FieldData::Double { .. } => ArrowDataType::Float64,
        FieldData::VarChar { .. }
        | FieldData::Json { .. }
        | FieldData::Geometry { .. }
        | FieldData::Timestamptz { .. } => ArrowDataType::Utf8,
        _ => ArrowDataType::Utf8,
    }
}

fn schema_from_field_data(fields: &[FieldData]) -> Schema {
    Schema::new(
        fields
            .iter()
            .map(|f| Field::new(f.name(), field_data_arrow_type(f), true))
            .collect::<Vec<_>>(),
    )
}

fn field_data_len(field: &FieldData) -> usize {
    match field {
        FieldData::Bool { values, .. } => values.len(),
        FieldData::Int8 { values, .. } => values.len(),
        FieldData::Int16 { values, .. } => values.len(),
        FieldData::Int32 { values, .. } => values.len(),
        FieldData::Int64 { values, .. } => values.len(),
        FieldData::Float { values, .. } => values.len(),
        FieldData::Double { values, .. } => values.len(),
        FieldData::VarChar { values, .. } => values.len(),
        FieldData::Json { values, .. } => values.len(),
        FieldData::Geometry { values, .. } => values.len(),
        FieldData::Timestamptz { values, .. } => values.len(),
        _ => 0,
    }
}

fn field_data_to_array(field: &FieldData, offset: usize, end: usize) -> Result<ArrayRef, ConnectorError> {
    match field {
        FieldData::Bool { values, .. } => Ok(Arc::new(BooleanArray::from(values[offset..end].to_vec()))),
        FieldData::Int8 { values, .. } => Ok(Arc::new(Int8Array::from(values[offset..end].to_vec()))),
        FieldData::Int16 { values, .. } => Ok(Arc::new(Int16Array::from(values[offset..end].to_vec()))),
        FieldData::Int32 { values, .. } => Ok(Arc::new(Int32Array::from(values[offset..end].to_vec()))),
        FieldData::Int64 { values, .. } => Ok(Arc::new(Int64Array::from(values[offset..end].to_vec()))),
        FieldData::Float { values, .. } => Ok(Arc::new(Float32Array::from(values[offset..end].to_vec()))),
        FieldData::Double { values, .. } => Ok(Arc::new(Float64Array::from(values[offset..end].to_vec()))),
        FieldData::VarChar { values, .. } => {
            let slice: Vec<&str> = values[offset..end].iter().map(|s| s.as_str()).collect();
            Ok(Arc::new(StringArray::from(slice)))
        },
        FieldData::Json { values, .. } => {
            let slice: Vec<String> = values[offset..end].iter().map(|v| v.to_string()).collect();
            let refs: Vec<&str> = slice.iter().map(|s| s.as_str()).collect();
            Ok(Arc::new(StringArray::from(refs)))
        },
        FieldData::Geometry { values, .. } | FieldData::Timestamptz { values, .. } => {
            let slice: Vec<&str> = values[offset..end].iter().map(|s| s.as_str()).collect();
            Ok(Arc::new(StringArray::from(slice)))
        },
        _ => Err(ConnectorError::InvalidRequest(format!(
            "Unsupported Milvus field type for field '{}'",
            field.name()
        ))),
    }
}

fn fields_to_batch(
    schema: &Arc<Schema>,
    fields: &[FieldData],
    offset: usize,
    end: usize,
) -> Result<RecordBatch, ConnectorError> {
    let arrays: Vec<ArrayRef> = fields
        .iter()
        .map(|f| field_data_to_array(f, offset, end))
        .collect::<Result<_, _>>()?;

    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(|e| ConnectorError::SQLError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_extract_credentials_success() {
        let conn = DataConnectionResource {
            metadata: commons::api::ResourceMetadata {
                id: "conn-1".to_string(),
                tenant_id: Some("tenant-1".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: commons::api::connections::DataConnection {
                name: "test-milvus".to_string(),
                data_connection_type_id: "milvus-type".to_string(),
                format: commons::api::connections::DataFormat::Tabular,
                admin: Some(Admin::Secret {
                    name: "test-milvus".to_string(),
                    secret: Arc::new(HashMap::from([
                        (KEY_HOST.to_string(), "localhost".to_string()),
                        (KEY_PORT.to_string(), "19530".to_string()),
                        (KEY_TOKEN.to_string(), "root:milvus".to_string()),
                        (KEY_DATABASE.to_string(), "default".to_string()),
                    ])),
                }),
                properties: HashMap::new(),
            },
            status: Default::default(),
        };
        let result = extract_credentials(&conn);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get(KEY_HOST).unwrap(), "localhost");
    }

    #[test]
    fn test_extract_credentials_missing() {
        let conn = DataConnectionResource {
            metadata: commons::api::ResourceMetadata {
                id: "conn-1".to_string(),
                tenant_id: Some("tenant-1".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: commons::api::connections::DataConnection {
                name: "test-milvus".to_string(),
                data_connection_type_id: "milvus-type".to_string(),
                format: commons::api::connections::DataFormat::Tabular,
                admin: None,
                properties: HashMap::new(),
            },
            status: Default::default(),
        };
        let result = extract_credentials(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_ids_single_int() {
        let v = serde_json::json!(42);
        let ids = parse_ids(&v).unwrap();
        match ids {
            Ids::Int64(v) => assert_eq!(v, vec![42]),
            _ => panic!("expected int ids"),
        }
    }

    #[test]
    fn test_parse_ids_array_int() {
        let v = serde_json::json!([1, 2, 3]);
        let ids = parse_ids(&v).unwrap();
        match ids {
            Ids::Int64(v) => assert_eq!(v, vec![1, 2, 3]),
            _ => panic!("expected int ids"),
        }
    }

    #[test]
    fn test_parse_ids_array_string() {
        let v = serde_json::json!(["a", "b", "c"]);
        let ids = parse_ids(&v).unwrap();
        match ids {
            Ids::VarChar(v) => assert_eq!(v, vec!["a", "b", "c"]),
            _ => panic!("expected string ids"),
        }
    }

    #[test]
    fn test_parse_ids_empty_array() {
        let v = serde_json::json!([]);
        assert!(parse_ids(&v).is_err());
    }

    #[test]
    fn test_field_data_arrow_type() {
        let f = |fd: FieldData| field_data_arrow_type(&fd);
        assert_eq!(
            f(FieldData::Bool {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Boolean
        );
        assert_eq!(
            f(FieldData::Int8 {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Int8
        );
        assert_eq!(
            f(FieldData::Int16 {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Int16
        );
        assert_eq!(
            f(FieldData::Int32 {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Int32
        );
        assert_eq!(
            f(FieldData::Int64 {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Int64
        );
        assert_eq!(
            f(FieldData::Float {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Float32
        );
        assert_eq!(
            f(FieldData::Double {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Float64
        );
        assert_eq!(
            f(FieldData::VarChar {
                name: "a".into(),
                values: vec![]
            }),
            ArrowDataType::Utf8
        );
    }

    #[test]
    fn test_field_data_len() {
        let field = FieldData::Int64 {
            name: "id".to_string(),
            values: vec![1, 2, 3],
        };
        assert_eq!(field_data_len(&field), 3);

        let field = FieldData::VarChar {
            name: "name".to_string(),
            values: vec!["a".to_string(), "b".to_string()],
        };
        assert_eq!(field_data_len(&field), 2);
    }

    #[test]
    fn test_field_data_to_array_int64() {
        let field = FieldData::Int64 {
            name: "id".to_string(),
            values: vec![10, 20, 30],
        };
        let array = field_data_to_array(&field, 0, 3).unwrap();
        let int_arr = array.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(int_arr.value(0), 10);
        assert_eq!(int_arr.value(2), 30);
    }

    #[test]
    fn test_field_data_to_array_varchar() {
        let field = FieldData::VarChar {
            name: "name".to_string(),
            values: vec!["alice".to_string(), "bob".to_string()],
        };
        let array = field_data_to_array(&field, 0, 2).unwrap();
        let str_arr = array.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(str_arr.value(0), "alice");
        assert_eq!(str_arr.value(1), "bob");
    }

    #[test]
    fn test_field_data_to_array_slice() {
        let field = FieldData::Int32 {
            name: "val".to_string(),
            values: vec![1, 2, 3, 4, 5],
        };
        let array = field_data_to_array(&field, 1, 4).unwrap();
        let int_arr = array.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(int_arr.len(), 3);
        assert_eq!(int_arr.value(0), 2);
        assert_eq!(int_arr.value(2), 4);
    }

    #[test]
    fn test_fields_to_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", ArrowDataType::Int64, true),
            Field::new("name", ArrowDataType::Utf8, true),
        ]));
        let fields = vec![
            FieldData::Int64 {
                name: "id".to_string(),
                values: vec![1, 2],
            },
            FieldData::VarChar {
                name: "name".to_string(),
                values: vec!["a".to_string(), "b".to_string()],
            },
        ];
        let batch = fields_to_batch(&schema, &fields, 0, 2).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);
    }
}
