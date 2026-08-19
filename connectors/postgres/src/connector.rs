use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    StringArray, Time64MicrosecondArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::errors::ConnectorError;
use commons::api::tabular::{QueryOptions, TabularState};
use commons::api::tabular::{QueryOutput, TabularReader};

use futures::StreamExt;

use commons::api::tabular::FlightConnector;
use moka::future::Cache;
use sqlx::Acquire;
use sqlx::postgres::PgRow;
use sqlx::{Column, Executor, PgPool, Row, Statement, TypeInfo};
use std::time::Duration;

pub struct PgConnector {
    pools: Cache<String, PgPool>,
}

impl PgConnector {
    pub fn new(cache_ttl: Duration, cache_idle: Duration, cache_max_capacity: u64) -> Self {
        Self {
            pools: Cache::builder()
                .time_to_live(cache_ttl)
                .time_to_idle(cache_idle)
                .max_capacity(cache_max_capacity)
                .build(),
        }
    }
}

#[async_trait::async_trait]
impl FlightConnector for PgConnector {
    fn provider(&self) -> String {
        "postgres".to_string()
    }

    fn description(&self) -> String {
        "PostgreSQL connector".to_string()
    }

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
    ) -> Result<Arc<dyn TabularReader>, ConnectorError> {
        let credentials = match &data_connection.resource.admin {
            Some(Admin::Secret { name: _, secret }) => Some(secret.clone()),
            _ => None,
        }
        .ok_or_else(|| ConnectorError::ConnectionError("PostgreSQL credentials are required".to_string()))?;

        let url = credentials
            .get("url")
            .ok_or_else(|| ConnectorError::ConnectionError("PostgreSQL URL is required".to_string()))?;

        let pool = self
            .pools
            .try_get_with(url.clone(), async {
                PgPool::connect(url.as_str())
                    .await
                    .map_err(|_| ConnectorError::ConnectionError("Failed to connect to PostgreSQL".to_string()))
            })
            .await
            .map_err(|_| ConnectorError::ConnectionError("Failed to get PostgreSQL reader".to_string()))?;

        Ok(Arc::new(PgReader { pool }))
    }
}

pub struct PgReader {
    pool: PgPool,
}

impl PgReader {}

#[async_trait::async_trait]
impl TabularReader for PgReader {
    fn provider(&self) -> String {
        "postgres".to_string()
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
            .map(|col| Field::new(col.name(), pg_type_to_arrow(col.type_info().name()), true))
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
            let mut tx = conn.begin().await.map_err(|e| ConnectorError::SQLError(e.to_string()))?;
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(|e| ConnectorError::SQLError(e.to_string()))?;

            {
                let mut rows = sqlx::query(query.as_str()).fetch(&mut *tx);
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
            }

            tx.commit().await.map_err(|e| ConnectorError::SQLError(e.to_string()))?;
        };

        Ok(Box::pin(stream))
    }

    async fn test_connection(&self) -> Result<(), ConnectorError> {
        Ok(())
    }
}

fn rows_to_batch(schema: &Arc<Schema>, rows: &[PgRow]) -> Result<RecordBatch, ConnectorError> {
    let columns = rows[0].columns();
    let arrays: Vec<ArrayRef> = (0..columns.len())
        .map(|col_idx| {
            let col = &columns[col_idx];
            build_array(col.type_info().name(), rows, col_idx)
        })
        .collect::<Result<_, _>>()?;

    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(|e| ConnectorError::SQLError(e.to_string()))
}

fn pg_type_to_arrow(pg_type: &str) -> DataType {
    match pg_type {
        "BOOL" => DataType::Boolean,
        "INT2" | "SMALLINT" | "SMALLSERIAL" => DataType::Int16,
        "INT4" | "INT" | "INTEGER" | "SERIAL" => DataType::Int32,
        "INT8" | "BIGINT" | "BIGSERIAL" => DataType::Int64,
        "FLOAT4" | "REAL" => DataType::Float32,
        "FLOAT8" | "DOUBLE PRECISION" => DataType::Float64,
        "BYTEA" => DataType::Binary,
        "TIMESTAMP" => DataType::Timestamp(TimeUnit::Microsecond, None),
        "TIMESTAMPTZ" => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        "DATE" => DataType::Date32,
        "TIME" => DataType::Time64(TimeUnit::Microsecond),
        "NUMERIC" => DataType::Utf8,
        "UUID" | "JSON" | "JSONB" => DataType::Utf8,
        _ => DataType::Utf8,
    }
}

fn timestamp_to_micros(dt: chrono::NaiveDateTime) -> i64 {
    dt.and_utc().timestamp_micros()
}

fn timestamptz_to_micros(dt: chrono::DateTime<chrono::Utc>) -> i64 {
    dt.timestamp_micros()
}

fn date_to_days(d: chrono::NaiveDate) -> i32 {
    d.signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        .num_days() as i32
}

fn time_to_micros(t: chrono::NaiveTime) -> i64 {
    use chrono::Timelike;
    t.num_seconds_from_midnight() as i64 * 1_000_000 + t.nanosecond() as i64 / 1_000
}

fn build_array(pg_type: &str, rows: &[PgRow], col_idx: usize) -> Result<ArrayRef, ConnectorError> {
    let err = |e: sqlx::Error| ConnectorError::SQLError(e.to_string());
    match pg_type {
        "BOOL" => {
            let vals: Vec<Option<bool>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            Ok(Arc::new(BooleanArray::from(vals)))
        },
        "INT2" | "SMALLINT" | "SMALLSERIAL" => {
            let vals: Vec<Option<i16>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            Ok(Arc::new(Int16Array::from(vals)))
        },
        "INT4" | "INT" | "INTEGER" | "SERIAL" => {
            let vals: Vec<Option<i32>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            Ok(Arc::new(Int32Array::from(vals)))
        },
        "INT8" | "BIGINT" | "BIGSERIAL" => {
            let vals: Vec<Option<i64>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            Ok(Arc::new(Int64Array::from(vals)))
        },
        "FLOAT4" | "REAL" => {
            let vals: Vec<Option<f32>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            Ok(Arc::new(Float32Array::from(vals)))
        },
        "FLOAT8" | "DOUBLE PRECISION" => {
            let vals: Vec<Option<f64>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            Ok(Arc::new(Float64Array::from(vals)))
        },
        "BYTEA" => {
            let vals: Vec<Option<Vec<u8>>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let vals: Vec<Option<&[u8]>> = vals.iter().map(|v| v.as_deref()).collect();
            Ok(Arc::new(BinaryArray::from(vals)))
        },
        "TIMESTAMP" => {
            let vals: Vec<Option<chrono::NaiveDateTime>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let micros: Vec<Option<i64>> = vals.iter().map(|v| v.map(timestamp_to_micros)).collect();
            Ok(Arc::new(TimestampMicrosecondArray::from(micros)))
        },
        "TIMESTAMPTZ" => {
            let vals: Vec<Option<chrono::DateTime<chrono::Utc>>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let micros: Vec<Option<i64>> = vals.iter().map(|v| v.map(timestamptz_to_micros)).collect();
            Ok(Arc::new(TimestampMicrosecondArray::from(micros).with_timezone("UTC")))
        },
        "DATE" => {
            let vals: Vec<Option<chrono::NaiveDate>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let days: Vec<Option<i32>> = vals.iter().map(|v| v.map(date_to_days)).collect();
            Ok(Arc::new(Date32Array::from(days)))
        },
        "TIME" => {
            let vals: Vec<Option<chrono::NaiveTime>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let micros: Vec<Option<i64>> = vals.iter().map(|v| v.map(time_to_micros)).collect();
            Ok(Arc::new(Time64MicrosecondArray::from(micros)))
        },
        "UUID" => {
            let vals: Vec<Option<uuid::Uuid>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let strs: Vec<Option<String>> = vals.iter().map(|v| v.map(|u| u.to_string())).collect();
            Ok(Arc::new(StringArray::from(strs)))
        },
        "JSON" | "JSONB" => {
            let vals: Vec<Option<serde_json::Value>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let strs: Vec<Option<String>> = vals.into_iter().map(|v| v.map(|j| j.to_string())).collect();
            Ok(Arc::new(StringArray::from(strs)))
        },
        "NUMERIC" => {
            let vals: Vec<Option<bigdecimal::BigDecimal>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            let strs: Vec<Option<String>> = vals.into_iter().map(|v| v.map(|d| d.to_string())).collect();
            Ok(Arc::new(StringArray::from(strs)))
        },
        _ => {
            let vals: Vec<Option<String>> = rows
                .iter()
                .map(|r| r.try_get(col_idx))
                .collect::<Result<_, _>>()
                .map_err(err)?;
            Ok(Arc::new(StringArray::from(vals)))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pg_type_to_arrow_bool() {
        assert_eq!(pg_type_to_arrow("BOOL"), DataType::Boolean);
    }

    #[test]
    fn test_pg_type_to_arrow_int16() {
        assert_eq!(pg_type_to_arrow("INT2"), DataType::Int16);
        assert_eq!(pg_type_to_arrow("SMALLINT"), DataType::Int16);
        assert_eq!(pg_type_to_arrow("SMALLSERIAL"), DataType::Int16);
    }

    #[test]
    fn test_pg_type_to_arrow_int32() {
        assert_eq!(pg_type_to_arrow("INT4"), DataType::Int32);
        assert_eq!(pg_type_to_arrow("INT"), DataType::Int32);
        assert_eq!(pg_type_to_arrow("INTEGER"), DataType::Int32);
        assert_eq!(pg_type_to_arrow("SERIAL"), DataType::Int32);
    }

    #[test]
    fn test_pg_type_to_arrow_int64() {
        assert_eq!(pg_type_to_arrow("INT8"), DataType::Int64);
        assert_eq!(pg_type_to_arrow("BIGINT"), DataType::Int64);
        assert_eq!(pg_type_to_arrow("BIGSERIAL"), DataType::Int64);
    }

    #[test]
    fn test_pg_type_to_arrow_float32() {
        assert_eq!(pg_type_to_arrow("FLOAT4"), DataType::Float32);
        assert_eq!(pg_type_to_arrow("REAL"), DataType::Float32);
    }

    #[test]
    fn test_pg_type_to_arrow_float64() {
        assert_eq!(pg_type_to_arrow("FLOAT8"), DataType::Float64);
        assert_eq!(pg_type_to_arrow("DOUBLE PRECISION"), DataType::Float64);
    }

    #[test]
    fn test_pg_type_to_arrow_binary() {
        assert_eq!(pg_type_to_arrow("BYTEA"), DataType::Binary);
    }

    #[test]
    fn test_pg_type_to_arrow_timestamp() {
        assert_eq!(
            pg_type_to_arrow("TIMESTAMP"),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
    }

    #[test]
    fn test_pg_type_to_arrow_timestamptz() {
        assert_eq!(
            pg_type_to_arrow("TIMESTAMPTZ"),
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
    }

    #[test]
    fn test_pg_type_to_arrow_date() {
        assert_eq!(pg_type_to_arrow("DATE"), DataType::Date32);
    }

    #[test]
    fn test_pg_type_to_arrow_time() {
        assert_eq!(pg_type_to_arrow("TIME"), DataType::Time64(TimeUnit::Microsecond));
    }

    #[test]
    fn test_pg_type_to_arrow_fallback() {
        assert_eq!(pg_type_to_arrow("TEXT"), DataType::Utf8);
        assert_eq!(pg_type_to_arrow("VARCHAR"), DataType::Utf8);
        assert_eq!(pg_type_to_arrow("UUID"), DataType::Utf8);
        assert_eq!(pg_type_to_arrow("JSON"), DataType::Utf8);
        assert_eq!(pg_type_to_arrow("JSONB"), DataType::Utf8);
        assert_eq!(pg_type_to_arrow("NUMERIC"), DataType::Utf8);
    }

    #[test]
    fn test_pg_connector_new() {
        let connector = PgConnector::new(Duration::from_secs(300), Duration::from_secs(60), 100);
        assert_eq!(connector.provider(), "postgres");
    }

    #[test]
    fn test_timestamp_to_micros() {
        let dt = chrono::NaiveDate::from_ymd_opt(2025, 6, 15)
            .unwrap()
            .and_hms_micro_opt(10, 30, 0, 123_456)
            .unwrap();
        assert_eq!(timestamp_to_micros(dt), 1_749_983_400_123_456);
    }

    #[test]
    fn test_timestamp_to_micros_epoch() {
        let dt = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(timestamp_to_micros(dt), 0);
    }

    #[test]
    fn test_timestamp_to_micros_before_epoch() {
        let dt = chrono::NaiveDate::from_ymd_opt(1969, 12, 31)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap();
        assert_eq!(timestamp_to_micros(dt), -1_000_000);
    }

    #[test]
    fn test_timestamptz_to_micros() {
        use chrono::TimeZone;
        let dt = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 10, 30, 0).unwrap();
        assert_eq!(timestamptz_to_micros(dt), 1_749_983_400_000_000);
    }

    #[test]
    fn test_date_to_days() {
        let d = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        assert_eq!(date_to_days(d), 0);

        let d = chrono::NaiveDate::from_ymd_opt(2025, 6, 15).unwrap();
        assert_eq!(date_to_days(d), 20_254);

        let d = chrono::NaiveDate::from_ymd_opt(1969, 12, 31).unwrap();
        assert_eq!(date_to_days(d), -1);
    }

    #[test]
    fn test_time_to_micros() {
        let t = chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        assert_eq!(time_to_micros(t), 0);

        let t = chrono::NaiveTime::from_hms_micro_opt(13, 45, 30, 123_456).unwrap();
        assert_eq!(time_to_micros(t), 49_530_123_456);

        let t = chrono::NaiveTime::from_hms_micro_opt(23, 59, 59, 999_999).unwrap();
        assert_eq!(time_to_micros(t), 86_399_999_999);
    }
}
