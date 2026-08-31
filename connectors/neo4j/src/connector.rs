use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float64Array, Int64Array, StringArray, TimestampMillisecondArray,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use commons::api::connections::DataConnectionResource;
use commons::api::connector::CredentialsResolver;
use commons::api::connector::{DataReader, FlightConnector, Query, QueryOptions, QueryOutput};
use commons::api::errors::ConnectorError;
use commons::utils::config::ConnectorConfig;
use futures::Stream;
use moka::future::Cache;
use neo4rs::{BoltType, Graph};

use crate::types;

const KEY_URI: &str = "NEO4J_URI";
const KEY_USERNAME: &str = "NEO4J_USERNAME";
const KEY_PASSWORD: &str = "NEO4J_PASSWORD";
const KEY_DATABASE: &str = "NEO4J_DATABASE";

pub struct Neo4jConnector {
    graphs: Cache<String, Graph>,
    config: ConnectorConfig,
}

impl Neo4jConnector {
    pub fn new(cache_ttl: Duration, cache_idle: Duration, cache_max_capacity: u64, config: ConnectorConfig) -> Self {
        Self {
            graphs: Cache::builder()
                .time_to_live(cache_ttl)
                .time_to_idle(cache_idle)
                .max_capacity(cache_max_capacity)
                .build(),
            config,
        }
    }
}

async fn build_graph(
    credentials: &HashMap<String, String>,
    connection_timeout: Duration,
) -> Result<Graph, ConnectorError> {
    let uri = credentials
        .get(KEY_URI)
        .ok_or_else(|| ConnectorError::ConnectionError("NEO4J_URI is required".to_string()))?;
    let username = credentials
        .get(KEY_USERNAME)
        .cloned()
        .unwrap_or_else(|| "neo4j".to_string());
    let password = credentials
        .get(KEY_PASSWORD)
        .ok_or_else(|| ConnectorError::ConnectionError("NEO4J_PASSWORD is required".to_string()))?;
    let database = credentials
        .get(KEY_DATABASE)
        .cloned()
        .unwrap_or_else(|| "neo4j".to_string());

    let config = neo4rs::ConfigBuilder::default()
        .uri(uri)
        .user(&username)
        .password(password)
        .db(database.as_str())
        .build()
        .map_err(|e| ConnectorError::ConnectionError(format!("Invalid Neo4j config: {e}")))?;

    tokio::time::timeout(connection_timeout, Graph::connect(config))
        .await
        .map_err(|_| ConnectorError::ConnectionError("Connection timeout".to_string()))?
        .map_err(|e| ConnectorError::ConnectionError(format!("Failed to connect to Neo4j: {e}")))
}

const PROVIDER: &str = "neo4j";

#[async_trait::async_trait]
impl FlightConnector for Neo4jConnector {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    fn description(&self) -> String {
        "Neo4j graph database connector".to_string()
    }

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
        credentials_resolver: &dyn CredentialsResolver,
    ) -> Result<Arc<dyn DataReader>, ConnectorError> {
        let connection_timeout = self.config.connection_timeout();

        let cache_key = data_connection.metadata.id.clone();

        let graph = self
            .graphs
            .try_get_with(cache_key, async {
                let credentials = credentials_resolver.resolve(data_connection).await?;
                build_graph(&credentials, connection_timeout).await
            })
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to get Neo4j client: {e}")))?;

        Ok(Arc::new(Neo4jReader { graph }))
    }
}

pub struct Neo4jReader {
    graph: Graph,
}

#[async_trait::async_trait]
impl DataReader for Neo4jReader {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<Query>, ConnectorError> {
        let mut result = self
            .graph
            .execute(neo4rs::query(query))
            .await
            .map_err(map_neo4j_error)?;

        let row = match result.next().await.map_err(map_neo4j_error)? {
            Some(row) => row,
            None => {
                return Ok(Arc::new(Query::new(query.to_owned(), Arc::new(Schema::empty()))));
            },
        };

        let json_row: serde_json::Value = row
            .to()
            .map_err(|e| ConnectorError::SQLError(format!("Failed to deserialize row: {e}")))?;

        let obj = json_row
            .as_object()
            .ok_or_else(|| ConnectorError::SQLError("Expected row to be a JSON object".to_string()))?;

        let fields: Vec<Field> = obj
            .keys()
            .map(|key| {
                let arrow_type = match row.get::<BoltType>(key) {
                    Ok(bt) => types::bolt_type_to_arrow(&bt),
                    Err(_) => ArrowDataType::Utf8,
                };
                Field::new(key, arrow_type, true)
            })
            .collect();

        Ok(Arc::new(Query::new(query.to_owned(), Arc::new(Schema::new(fields)))))
    }

    async fn read_tabular(&self, query: Arc<Query>, options: &QueryOptions) -> QueryOutput {
        let graph = self.graph.clone();
        let schema = query.schema.clone();
        let query = query.query.clone();
        let batch_size = options.batch_size;

        #[allow(clippy::while_let_loop)]
        let stream = async_stream::try_stream! {
            let mut result = graph
                .execute(neo4rs::query(&query))
                .await
                .map_err(map_neo4j_error)?;

            let mut chunk: Vec<neo4rs::Row> = Vec::with_capacity(batch_size);

            loop {
                match result.next().await.map_err(map_neo4j_error)? {
                    Some(row) => {
                        chunk.push(row);
                        if chunk.len() >= batch_size {
                            yield rows_to_record_batch(&schema, &chunk)?;
                            chunk.clear();
                        }
                    }
                    None => break,
                }
            }

            if !chunk.is_empty() {
                yield rows_to_record_batch(&schema, &chunk)?;
            }
        };

        Ok(Box::pin(stream)
            as Pin<
                Box<dyn Stream<Item = Result<RecordBatch, ConnectorError>> + Send>,
            >)
    }

    async fn check_connection(&self) -> Result<(), ConnectorError> {
        let mut result = self
            .graph
            .execute(neo4rs::query("RETURN 1"))
            .await
            .map_err(map_neo4j_error)?;

        result.next().await.map_err(map_neo4j_error)?;
        Ok(())
    }
}

fn map_neo4j_error(e: neo4rs::Error) -> ConnectorError {
    let msg = e.to_string().to_lowercase();
    if msg.contains("permission")
        || msg.contains("not allowed")
        || msg.contains("read only")
        || msg.contains("write operations are not allowed")
    {
        return ConnectorError::InvalidRequest("Data source is read-only".to_string());
    }
    ConnectorError::ConnectionError("Neo4j connection error".to_string())
}

fn rows_to_record_batch(schema: &Arc<Schema>, rows: &[neo4rs::Row]) -> Result<RecordBatch, ConnectorError> {
    let arrays: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|field| build_column_array(field.name(), field.data_type(), rows))
        .collect::<Result<_, _>>()?;

    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(|e| ConnectorError::SQLError(e.to_string()))
}

fn build_column_array(
    field_name: &str,
    data_type: &ArrowDataType,
    rows: &[neo4rs::Row],
) -> Result<ArrayRef, ConnectorError> {
    match data_type {
        ArrowDataType::Boolean => {
            let vals: Vec<Option<bool>> = rows
                .iter()
                .map(|r| r.get::<Option<bool>>(field_name).ok().flatten())
                .collect();
            Ok(Arc::new(BooleanArray::from(vals)))
        },
        ArrowDataType::Int64 => {
            let vals: Vec<Option<i64>> = rows
                .iter()
                .map(|r| r.get::<Option<i64>>(field_name).ok().flatten())
                .collect();
            Ok(Arc::new(Int64Array::from(vals)))
        },
        ArrowDataType::Float64 => {
            let vals: Vec<Option<f64>> = rows
                .iter()
                .map(|r| r.get::<Option<f64>>(field_name).ok().flatten())
                .collect();
            Ok(Arc::new(Float64Array::from(vals)))
        },
        ArrowDataType::Date32 => {
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            let vals: Vec<Option<i32>> = rows
                .iter()
                .map(|r| {
                    r.get::<chrono::NaiveDate>(field_name)
                        .ok()
                        .map(|d| (d - epoch).num_days() as i32)
                })
                .collect();
            Ok(Arc::new(Date32Array::from(vals)))
        },
        ArrowDataType::Timestamp(TimeUnit::Millisecond, tz) => {
            let vals: Vec<Option<i64>> = rows
                .iter()
                .map(|r| {
                    r.get::<BoltType>(field_name).ok().and_then(|bt| match bt {
                        BoltType::DateTime(dt) => {
                            let chrono_dt: chrono::DateTime<chrono::FixedOffset> = dt.try_into().ok()?;
                            Some(chrono_dt.timestamp_millis())
                        },
                        BoltType::DateTimeZoneId(_) => r
                            .get::<chrono::DateTime<chrono::FixedOffset>>(field_name)
                            .ok()
                            .map(|dt| dt.timestamp_millis()),
                        BoltType::LocalDateTime(dt) => {
                            let chrono_dt: chrono::NaiveDateTime = dt.try_into().ok()?;
                            Some(chrono_dt.and_utc().timestamp_millis())
                        },
                        _ => None,
                    })
                })
                .collect();
            let arr = TimestampMillisecondArray::from(vals);
            if let Some(tz) = tz {
                Ok(Arc::new(arr.with_timezone(tz.as_ref())))
            } else {
                Ok(Arc::new(arr))
            }
        },
        ArrowDataType::Binary => {
            let bolt_vals: Vec<Option<Vec<u8>>> = rows.iter().map(|r| r.get::<Vec<u8>>(field_name).ok()).collect();
            let refs: Vec<Option<&[u8]>> = bolt_vals.iter().map(|v| v.as_deref()).collect();
            Ok(Arc::new(BinaryArray::from(refs)))
        },
        _ => {
            let vals: Vec<Option<String>> = rows.iter().map(|r| extract_as_string(r, field_name)).collect();
            Ok(Arc::new(StringArray::from(vals)))
        },
    }
}

fn extract_as_string(row: &neo4rs::Row, key: &str) -> Option<String> {
    if let Ok(Some(s)) = row.get::<Option<String>>(key) {
        return Some(s);
    }
    if let Ok(bolt) = row.get::<BoltType>(key) {
        return match bolt {
            BoltType::Null(_) => None,
            other => Some(types::bolt_value_to_json(&other).to_string()),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;

    #[test]
    fn test_connector_provider() {
        let connector = Neo4jConnector::new(
            Duration::from_secs(300),
            Duration::from_secs(60),
            100,
            ConnectorConfig::default(),
        );
        assert_eq!(connector.provider(), "neo4j");
    }

    #[test]
    fn test_connector_description() {
        let connector = Neo4jConnector::new(
            Duration::from_secs(300),
            Duration::from_secs(60),
            100,
            ConnectorConfig::default(),
        );
        assert_eq!(connector.description(), "Neo4j graph database connector");
    }

    #[test]
    fn test_map_neo4j_error_permission_denied() {
        let err = map_neo4j_error(neo4rs::Error::UnsupportedScheme("permission denied".into()));
        assert!(matches!(err, ConnectorError::InvalidRequest(_)));
    }

    #[test]
    fn test_build_column_array_boolean() {
        let fields = neo4rs::BoltList::from(vec![BoltType::String("active".into())]);
        let data1 = neo4rs::BoltList::from(vec![BoltType::Boolean(neo4rs::BoltBoolean { value: true })]);
        let data2 = neo4rs::BoltList::from(vec![BoltType::Boolean(neo4rs::BoltBoolean { value: false })]);
        let rows = vec![
            neo4rs::Row::new(fields.clone(), data1),
            neo4rs::Row::new(fields.clone(), data2),
        ];
        let arr = build_column_array("active", &ArrowDataType::Boolean, &rows).unwrap();
        let bool_arr = arr.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert_eq!(bool_arr.len(), 2);
        assert!(bool_arr.value(0));
        assert!(!bool_arr.value(1));
    }

    #[test]
    fn test_build_column_array_int64() {
        let fields = neo4rs::BoltList::from(vec![BoltType::String("count".into())]);
        let data1 = neo4rs::BoltList::from(vec![BoltType::Integer(neo4rs::BoltInteger { value: 42 })]);
        let data2 = neo4rs::BoltList::from(vec![BoltType::Null(neo4rs::BoltNull)]);
        let rows = vec![
            neo4rs::Row::new(fields.clone(), data1),
            neo4rs::Row::new(fields.clone(), data2),
        ];
        let arr = build_column_array("count", &ArrowDataType::Int64, &rows).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(int_arr.value(0), 42);
        assert!(int_arr.is_null(1));
    }

    #[test]
    fn test_build_column_array_float64() {
        let fields = neo4rs::BoltList::from(vec![BoltType::String("score".into())]);
        let data = neo4rs::BoltList::from(vec![BoltType::Float(neo4rs::BoltFloat { value: 2.72 })]);
        let rows = vec![neo4rs::Row::new(fields, data)];
        let arr = build_column_array("score", &ArrowDataType::Float64, &rows).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((f_arr.value(0) - 2.72).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_column_array_utf8_fallback() {
        let fields = neo4rs::BoltList::from(vec![BoltType::String("name".into())]);
        let data = neo4rs::BoltList::from(vec![BoltType::String("Alice".into())]);
        let rows = vec![neo4rs::Row::new(fields, data)];
        let arr = build_column_array("name", &ArrowDataType::Utf8, &rows).unwrap();
        let str_arr = arr.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(str_arr.value(0), "Alice");
    }

    #[test]
    fn test_rows_to_record_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, true),
            Field::new("age", ArrowDataType::Int64, true),
        ]));
        let fields = neo4rs::BoltList::from(vec![BoltType::String("name".into()), BoltType::String("age".into())]);
        let data1 = neo4rs::BoltList::from(vec![
            BoltType::String("Alice".into()),
            BoltType::Integer(neo4rs::BoltInteger { value: 30 }),
        ]);
        let data2 = neo4rs::BoltList::from(vec![
            BoltType::String("Bob".into()),
            BoltType::Integer(neo4rs::BoltInteger { value: 25 }),
        ]);
        let rows = vec![
            neo4rs::Row::new(fields.clone(), data1),
            neo4rs::Row::new(fields.clone(), data2),
        ];
        let batch = rows_to_record_batch(&schema, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);

        let name_arr = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(name_arr.value(0), "Alice");
        assert_eq!(name_arr.value(1), "Bob");

        let age_arr = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(age_arr.value(0), 30);
        assert_eq!(age_arr.value(1), 25);
    }
}
