use std::sync::Arc;

use arrow::array::{ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::errors::ConnectorError;
use commons::api::tabular::{QueryOptions, TabularState};
use commons::api::tabular::{QueryOutput, TabularReader};

use futures::StreamExt;

use commons::api::tabular::FlightConnector;
use moka::future::Cache;
use sqlx::sqlite::SqliteRow;
use sqlx::{Column, Executor, Row, SqlitePool, Statement, TypeInfo};

pub struct SqliteConnector {
    pools: Cache<String, SqlitePool>,
}

impl Default for SqliteConnector {
    fn default() -> Self {
        Self {
            pools: Cache::builder().max_capacity(2).build(),
        }
    }
}

impl SqliteConnector {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl FlightConnector for SqliteConnector {
    fn provider(&self) -> String {
        "sqlite".to_string()
    }

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
    ) -> Result<Arc<dyn TabularReader>, ConnectorError> {
        let credentials = match &data_connection.resource.admin {
            Some(Admin::Secret { secret }) => Some(secret.clone()),
            _ => None,
        }
        .ok_or_else(|| ConnectorError::ConnectionError("SQLite credentials are required".to_string()))?;

        let url = credentials
            .get("url")
            .ok_or_else(|| ConnectorError::ConnectionError("SQLite URL is required".to_string()))?;
        let pool = self
            .pools
            .try_get_with(url.clone(), async {
                SqlitePool::connect(url.as_str())
                    .await
                    .map_err(|_| ConnectorError::ConnectionError("Failed to connect to SQLite".to_string()))
            })
            .await
            .map_err(|_| ConnectorError::ConnectionError("Failed to get SQLite reader".to_string()))?;

        Ok(Arc::new(SqliteReader { pool }))
    }
}

pub struct SqliteReader {
    pool: SqlitePool,
}

#[async_trait::async_trait]
impl TabularReader for SqliteReader {
    fn provider(&self) -> String {
        "sqlite".to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<TabularState>, ConnectorError> {
        let statement = self
            .pool
            .prepare(query)
            .await
            .map_err(|e| ConnectorError::SQLError(e.to_string()))?;

        let fields: Vec<Field> = statement
            .columns()
            .iter()
            .map(|col| Field::new(col.name(), sqlite_type_to_arrow(col.type_info().name()), true))
            .collect();

        Ok(Arc::new(TabularState::new(
            query.to_owned(),
            Arc::new(Schema::new(fields)),
        )))
    }

    async fn read(&self, state: Arc<TabularState>, options: &QueryOptions) -> QueryOutput {
        let pool = self.pool.clone();
        let schema = state.schema.clone();
        let query = state.query.clone();
        let batch_size = options.batch_size;

        let stream = async_stream::try_stream! {
            let mut conn = pool.acquire().await.map_err(|e| ConnectorError::ConnectionError(e.to_string()))?;
            sqlx::query("PRAGMA query_only = ON")
                .execute(&mut *conn)
                .await
                .map_err(|e| ConnectorError::SQLError(e.to_string()))?;

            let mut rows = sqlx::query(query.as_str()).fetch(&mut *conn);
            let mut chunk = Vec::with_capacity(batch_size);

            while let Some(row) = rows.next().await {
                chunk.push(row.map_err(|e| ConnectorError::SQLError(e.to_string()))?);

                if chunk.len() >= batch_size {
                    yield rows_to_batch(&schema, &chunk)?;
                    chunk.clear();
                }
            }

            if !chunk.is_empty() {
                yield rows_to_batch(&schema, &chunk)?;
            }
        };

        Ok(Box::pin(stream))
    }
}

fn rows_to_batch(schema: &Arc<Schema>, rows: &[SqliteRow]) -> Result<RecordBatch, ConnectorError> {
    let columns = rows[0].columns();
    let arrays: Vec<ArrayRef> = (0..columns.len())
        .map(|col_idx| {
            let col = &columns[col_idx];
            build_array(col.type_info().name(), rows, col_idx)
        })
        .collect();

    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(|e| ConnectorError::SQLError(e.to_string()))
}

fn sqlite_type_to_arrow(sqlite_type: &str) -> DataType {
    match sqlite_type {
        "BOOLEAN" => DataType::Boolean,
        "INTEGER" => DataType::Int64,
        "REAL" => DataType::Float64,
        "BLOB" => DataType::Binary,
        _ => DataType::Utf8,
    }
}

fn build_array(sqlite_type: &str, rows: &[SqliteRow], col_idx: usize) -> ArrayRef {
    match sqlite_type {
        "BOOLEAN" => {
            let vals: Vec<Option<bool>> = rows.iter().map(|r| r.get(col_idx)).collect();
            Arc::new(BooleanArray::from(vals))
        },
        "INTEGER" => {
            let vals: Vec<Option<i64>> = rows.iter().map(|r| r.get(col_idx)).collect();
            Arc::new(Int64Array::from(vals))
        },
        "REAL" => {
            let vals: Vec<Option<f64>> = rows.iter().map(|r| r.get(col_idx)).collect();
            Arc::new(Float64Array::from(vals))
        },
        "BLOB" => {
            let vals: Vec<Option<Vec<u8>>> = rows.iter().map(|r| r.get(col_idx)).collect();
            let vals: Vec<Option<&[u8]>> = vals.iter().map(|v| v.as_deref()).collect();
            Arc::new(BinaryArray::from(vals))
        },
        _ => {
            let vals: Vec<Option<String>> = rows.iter().map(|r| r.get(col_idx)).collect();
            Arc::new(StringArray::from(vals))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_type_to_arrow_boolean() {
        assert_eq!(sqlite_type_to_arrow("BOOLEAN"), DataType::Boolean);
    }

    #[test]
    fn test_sqlite_type_to_arrow_integer() {
        assert_eq!(sqlite_type_to_arrow("INTEGER"), DataType::Int64);
    }

    #[test]
    fn test_sqlite_type_to_arrow_real() {
        assert_eq!(sqlite_type_to_arrow("REAL"), DataType::Float64);
    }

    #[test]
    fn test_sqlite_type_to_arrow_blob() {
        assert_eq!(sqlite_type_to_arrow("BLOB"), DataType::Binary);
    }

    #[test]
    fn test_sqlite_type_to_arrow_text() {
        assert_eq!(sqlite_type_to_arrow("TEXT"), DataType::Utf8);
    }

    #[test]
    fn test_sqlite_type_to_arrow_fallback() {
        assert_eq!(sqlite_type_to_arrow("VARCHAR"), DataType::Utf8);
        assert_eq!(sqlite_type_to_arrow("DATETIME"), DataType::Utf8);
        assert_eq!(sqlite_type_to_arrow("NUMERIC"), DataType::Utf8);
        assert_eq!(sqlite_type_to_arrow("NULL"), DataType::Utf8);
    }

    #[test]
    fn test_sqlite_connector_new() {
        let connector = SqliteConnector::new();
        assert_eq!(connector.provider(), "sqlite");
    }
}
