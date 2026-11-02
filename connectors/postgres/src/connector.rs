use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float32Array, Float32Builder, Float64Array, Int16Array,
    Int32Array, Int64Array, ListBuilder, StringArray, Time64MicrosecondArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{ArrowPrimitiveType, DataType, Field, Float16Type, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use commons::api::connections::DataConnectionResource;
use commons::api::connector::{DataReader, QueryOutput, TableInfo};
use commons::api::connector::{Query, QueryOptions};
use commons::api::errors::ConnectorError;

use futures::StreamExt;

use commons::api::connector::CredentialsResolver;
use commons::api::connector::FlightConnector;
use commons::utils::config::ConnectorConfig;
use moka::future::Cache;
use sqlx::Acquire;
use sqlx::postgres::{PgConnectOptions, PgRow, PgValueFormat};
use sqlx::{Column, Executor, PgPool, Row, Statement, TypeInfo, ValueRef};
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

const PG_READ_ONLY_SQL_TRANSACTION: &str = "25006";
const KEY_URI: &str = "URI";
const KEY_CA_CERT: &str = "CA_CERT";

fn map_sqlx_error(e: sqlx::Error) -> ConnectorError {
    if let sqlx::Error::Database(ref db_err) = e
        && db_err.code().as_deref() == Some(PG_READ_ONLY_SQL_TRANSACTION)
    {
        return ConnectorError::InvalidRequest("data source is read-only".to_string());
    }
    ConnectorError::SQLError(e.to_string())
}

pub struct PgConnector {
    pools: Cache<String, PgPool>,
    config: ConnectorConfig,
}

const PROVIDER: &str = "postgres";

impl PgConnector {
    pub fn new(cache_ttl: Duration, cache_idle: Duration, cache_max_capacity: u64, config: ConnectorConfig) -> Self {
        Self {
            pools: Cache::builder()
                .time_to_live(cache_ttl)
                .time_to_idle(cache_idle)
                .max_capacity(cache_max_capacity)
                .build(),
            config,
        }
    }
}
#[async_trait::async_trait]
impl FlightConnector for PgConnector {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    fn description(&self) -> String {
        "PostgreSQL connector".to_string()
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
        info!("Creating Postgres reader");

        let connection_timeout = self.config.connection_timeout();
        let cache_key = data_connection.metadata.id.clone();

        let pool = self
            .pools
            .try_get_with(cache_key, async {
                let credentials = credentials_resolver.resolve(data_connection).await?;
                let url = credentials
                    .get(KEY_URI)
                    .ok_or_else(|| ConnectorError::ConnectionError(format!("'{KEY_URI}' credential is required")))?;

                let mut options = PgConnectOptions::from_str(url.as_str())
                    .map_err(|e| ConnectorError::ConnectionError(format!("invalid PostgreSQL URL: {e}")))?;

                if let Some(ca_cert) = credentials.get(KEY_CA_CERT) {
                    options = options.ssl_root_cert_from_pem(ca_cert.as_bytes().to_vec());
                }

                sqlx::pool::PoolOptions::<sqlx::Postgres>::new()
                    .acquire_timeout(connection_timeout)
                    .connect_with(options)
                    .await
                    .map_err(|e| ConnectorError::ConnectionError(format!("unable to connect to PostgreSQL: {e}")))
            })
            .await
            .map_err(|e| Arc::try_unwrap(e).unwrap_or_else(|arc| (*arc).clone()))?;

        Ok(Arc::new(PgReader { pool }))
    }
}

pub struct PgReader {
    pool: PgPool,
}

impl PgReader {}

#[async_trait::async_trait]
impl DataReader for PgReader {
    fn provider(&self) -> String {
        PROVIDER.to_string()
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER))]
    async fn schema(&self, query: &str) -> Result<Arc<Query>, ConnectorError> {
        let statement = self.pool.prepare(query).await.map_err(map_sqlx_error)?;

        let fields: Vec<Field> = statement
            .columns()
            .iter()
            .map(|col| Field::new(col.name(), pg_type_to_arrow(col.type_info().name()), true))
            .collect();

        Ok(Arc::new(Query::new(query.to_owned(), Arc::new(Schema::new(fields)))))
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER, batch_size = options.batch_size))]
    async fn read_tabular(&self, query: Arc<Query>, options: &QueryOptions) -> QueryOutput {
        let pool = self.pool.clone();
        let schema = query.schema.clone();
        let query = query.query.clone();
        let batch_size = options.batch_size;

        let stream = async_stream::try_stream! {
            let mut conn = pool.acquire().await.map_err(|e| ConnectorError::ConnectionError(e.to_string()))?;
            let mut tx = conn.begin().await.map_err(map_sqlx_error)?;
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_error)?;

            {
                let mut rows = sqlx::query(query.as_str()).fetch(&mut *tx);
                let mut chunk = Vec::with_capacity(batch_size);

                while let Some(row) = rows.next().await {
                    chunk.push(row.map_err(map_sqlx_error)?);

                    if chunk.len() >= batch_size {
                        yield rows_to_batch(&schema, &chunk)?;
                        chunk.clear();
                    }
                }

                if !chunk.is_empty() {
                    yield rows_to_batch(&schema, &chunk)?;
                }
            }

            tx.commit().await.map_err(map_sqlx_error)?;
        };

        Ok(Box::pin(stream))
    }

    #[tracing::instrument(skip_all, fields(connector.provider = PROVIDER))]
    async fn check_connection(&self) -> Result<(), ConnectorError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|_| ConnectorError::ConnectionError("PostgreSQL connection check failed".to_string()))?;
        Ok(())
    }

    #[tracing::instrument(
        skip_all,
        fields(
        connector.provider = PROVIDER,
        include_schema = include_schema,
        row_count = tracing::field::Empty,
        )
    )]
    async fn list_tables(
        &self,
        table_name_filter: Option<&str>,
        include_schema: bool,
    ) -> Result<Vec<TableInfo>, ConnectorError> {
        let rows: Vec<(String, String, String, String)> = match table_name_filter {
            Some(pattern) => sqlx::query_as(
                "SELECT table_catalog, table_schema, table_name, table_type \
                     FROM information_schema.tables \
                     WHERE table_schema NOT IN ('information_schema', 'pg_catalog') \
                     AND table_name LIKE $1 \
                     ORDER BY table_schema, table_name",
            )
            .bind(pattern)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?,
            None => sqlx::query_as(
                "SELECT table_catalog, table_schema, table_name, table_type \
                     FROM information_schema.tables \
                     WHERE table_schema NOT IN ('information_schema', 'pg_catalog') \
                     ORDER BY table_schema, table_name",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?,
        };

        let mut tables = Vec::with_capacity(rows.len());
        for (catalog, schema_name, table_name, table_type) in rows {
            let table_schema = if include_schema {
                let qualified = format!(
                    "\"{}\".\"{}\"",
                    schema_name.replace('"', "\"\""),
                    table_name.replace('"', "\"\"")
                );
                let query = format!("SELECT * FROM {} LIMIT 0", qualified);
                let statement = self.pool.prepare(&query).await.map_err(map_sqlx_error)?;

                let fields: Vec<Field> = statement
                    .columns()
                    .iter()
                    .map(|col| Field::new(col.name(), pg_type_to_arrow(col.type_info().name()), true))
                    .collect();
                Schema::new(fields)
            } else {
                Schema::empty()
            };

            tables.push(TableInfo {
                catalog,
                schema_name,
                table_name,
                table_type,
                table_schema,
            });
        }

        tracing::Span::current().record("row_count", tables.len());
        Ok(tables)
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

/// Strip the schema qualifier and identifier quotes from a type name.
///
/// sqlx names extension types via `oid::regtype::text`, which is
/// schema-qualified (e.g. `ext.vector`, `"my schema".vector`) when the
/// type's schema is not on the session `search_path`. Built-in names never
/// contain a dot, so they pass through unchanged. Only a schema qualifier is
/// stripped; a quoted identifier that itself contains a dot (e.g. `"a.b"`)
/// is not supported.
fn unqualified_type_name(pg_type: &str) -> &str {
    pg_type
        .rsplit_once('.')
        .map_or(pg_type, |(_, name)| name)
        .trim_matches('"')
}

fn pg_type_to_arrow(pg_type: &str) -> DataType {
    match unqualified_type_name(pg_type) {
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
        // pgvector extension types. Dimension lives in the column typmod,
        // which sqlx does not expose, so a variable-size list is used.
        PGVECTOR_VECTOR | PGVECTOR_HALFVEC => pgvector_list_type(),
        PGVECTOR_SPARSEVEC => DataType::Utf8,
        _ => DataType::Utf8,
    }
}

// pgvector type names are lowercase, unlike the uppercase built-in names
// above: sqlx reports built-in types from its static table (`INT4`, `FLOAT8`)
// but names extension types by their `regtype` text, which is lowercase.
const PGVECTOR_VECTOR: &str = "vector";
const PGVECTOR_HALFVEC: &str = "halfvec";
const PGVECTOR_SPARSEVEC: &str = "sparsevec";

/// Arrow type for pgvector `vector` and `halfvec` columns. `halfvec` is
/// widened to `Float32` (lossless) for broad consumer compatibility.
fn pgvector_list_type() -> DataType {
    DataType::List(Arc::new(Field::new_list_field(DataType::Float32, true)))
}

fn read_be_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn read_be_i32(bytes: &[u8], offset: usize) -> i32 {
    read_be_u32(bytes, offset) as i32
}

fn check_len(type_name: &str, bytes: &[u8], expected: usize) -> Result<(), String> {
    if bytes.len() != expected {
        return Err(format!(
            "invalid {type_name} binary value: expected {expected} bytes, got {}",
            bytes.len()
        ));
    }
    Ok(())
}

/// Decode pgvector `vector` binary format: `int16 dim, int16 unused, float4[dim]`.
fn decode_pgvector(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() < 4 {
        return Err(format!("invalid vector binary value: {} bytes", bytes.len()));
    }
    let dim = read_be_u16(bytes, 0) as usize;
    check_len(PGVECTOR_VECTOR, bytes, 4 + dim * 4)?;
    Ok((0..dim)
        .map(|i| f32::from_bits(read_be_u32(bytes, 4 + i * 4)))
        .collect())
}

/// Decode pgvector `halfvec` binary format: `int16 dim, int16 unused, half[dim]`.
fn decode_pghalfvec(bytes: &[u8]) -> Result<Vec<f32>, String> {
    type F16 = <Float16Type as ArrowPrimitiveType>::Native;
    if bytes.len() < 4 {
        return Err(format!("invalid halfvec binary value: {} bytes", bytes.len()));
    }
    let dim = read_be_u16(bytes, 0) as usize;
    check_len(PGVECTOR_HALFVEC, bytes, 4 + dim * 2)?;
    Ok((0..dim)
        .map(|i| F16::from_bits(read_be_u16(bytes, 4 + i * 2)).to_f32())
        .collect())
}

/// Decode pgvector `sparsevec` binary format
/// (`int32 dim, int32 nnz, int32 unused, int32[nnz] indices, float4[nnz] values`)
/// into its text representation, e.g. `{1:1.5,3:2.5}/5` (1-based indices).
fn decode_pgsparsevec(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() < 12 {
        return Err(format!("invalid sparsevec binary value: {} bytes", bytes.len()));
    }
    let dim = read_be_i32(bytes, 0);
    let nnz = usize::try_from(read_be_i32(bytes, 4)).map_err(|_| "invalid sparsevec nnz".to_string())?;
    check_len(PGVECTOR_SPARSEVEC, bytes, 12 + nnz * 8)?;
    let values_offset = 12 + nnz * 4;
    let elements: Vec<String> = (0..nnz)
        .map(|i| {
            let index = read_be_i32(bytes, 12 + i * 4) as i64 + 1;
            let value = f32::from_bits(read_be_u32(bytes, values_offset + i * 4));
            format!("{index}:{value}")
        })
        .collect();
    Ok(format!("{{{}}}/{dim}", elements.join(",")))
}

/// Read the raw binary bytes of each row's column value, `None` for NULL.
///
/// Relies on rows being fetched in binary format: `sqlx::query()` always
/// carries (possibly empty) arguments, so sqlx executes it as a prepared
/// statement with binary result formats. The simple query protocol
/// (`sqlx::raw_sql`) returns text instead, which this rejects explicitly.
fn raw_binary_values(rows: &[PgRow], col_idx: usize) -> Result<Vec<Option<&[u8]>>, ConnectorError> {
    rows.iter()
        .map(|r| {
            let value = r.try_get_raw(col_idx).map_err(map_sqlx_error)?;
            if value.is_null() {
                return Ok(None);
            }
            if value.format() != PgValueFormat::Binary {
                return Err(ConnectorError::SQLError(format!(
                    "unexpected text format for column {col_idx} of type {}",
                    value.type_info().name()
                )));
            }
            value
                .as_bytes()
                .map(Some)
                .map_err(|e| ConnectorError::SQLError(e.to_string()))
        })
        .collect()
}

/// Decoder from a pgvector binary value to its `f32` elements.
type VectorDecoder = fn(&[u8]) -> Result<Vec<f32>, String>;

fn build_pgvector_list_array(
    rows: &[PgRow],
    col_idx: usize,
    decode: VectorDecoder,
) -> Result<ArrayRef, ConnectorError> {
    let mut builder = ListBuilder::new(Float32Builder::new());
    for value in raw_binary_values(rows, col_idx)? {
        match value {
            Some(bytes) => {
                let vals = decode(bytes).map_err(ConnectorError::SQLError)?;
                builder.values().append_slice(&vals);
                builder.append(true);
            },
            None => builder.append(false),
        }
    }
    Ok(Arc::new(builder.finish()))
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
    let err = map_sqlx_error;
    match unqualified_type_name(pg_type) {
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
        PGVECTOR_VECTOR => build_pgvector_list_array(rows, col_idx, decode_pgvector),
        PGVECTOR_HALFVEC => build_pgvector_list_array(rows, col_idx, decode_pghalfvec),
        PGVECTOR_SPARSEVEC => {
            let strs: Vec<Option<String>> = raw_binary_values(rows, col_idx)?
                .into_iter()
                .map(|v| v.map(decode_pgsparsevec).transpose())
                .collect::<Result<_, _>>()
                .map_err(ConnectorError::SQLError)?;
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
    fn test_pg_type_to_arrow_pgvector() {
        assert_eq!(pg_type_to_arrow("vector"), pgvector_list_type());
        assert_eq!(pg_type_to_arrow("halfvec"), pgvector_list_type());
        assert_eq!(pg_type_to_arrow("sparsevec"), DataType::Utf8);
    }

    #[test]
    fn test_pg_type_to_arrow_pgvector_schema_qualified() {
        assert_eq!(pg_type_to_arrow("public.vector"), pgvector_list_type());
        assert_eq!(pg_type_to_arrow("ext.halfvec"), pgvector_list_type());
        assert_eq!(pg_type_to_arrow("\"my schema\".vector"), pgvector_list_type());
        assert_eq!(pg_type_to_arrow("\"My.Schema\".halfvec"), pgvector_list_type());
    }

    #[test]
    fn test_unqualified_type_name() {
        assert_eq!(unqualified_type_name("INT4"), "INT4");
        assert_eq!(unqualified_type_name("DOUBLE PRECISION"), "DOUBLE PRECISION");
        assert_eq!(unqualified_type_name("vector"), "vector");
        assert_eq!(unqualified_type_name("public.vector"), "vector");
        assert_eq!(unqualified_type_name("\"my schema\".vector"), "vector");
    }

    fn vector_bytes(vals: &[f32]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&(vals.len() as u16).to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        for v in vals {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b
    }

    #[test]
    fn test_decode_pgvector() {
        let vals = [0.1f32, -2.0, 3.5];
        assert_eq!(decode_pgvector(&vector_bytes(&vals)).unwrap(), vals);
    }

    #[test]
    fn test_decode_pgvector_empty() {
        assert_eq!(decode_pgvector(&vector_bytes(&[])).unwrap(), Vec::<f32>::new());
    }

    #[test]
    fn test_decode_pgvector_invalid_length() {
        let mut b = vector_bytes(&[1.0, 2.0]);
        b.pop();
        assert!(decode_pgvector(&b).is_err());
        assert!(decode_pgvector(&[0, 1]).is_err());
    }

    #[test]
    fn test_decode_pghalfvec() {
        // 1.0 = 0x3C00, -2.0 = 0xC000, 0.5 = 0x3800 in IEEE 754 half precision.
        let b = [0, 3, 0, 0, 0x3C, 0x00, 0xC0, 0x00, 0x38, 0x00];
        assert_eq!(decode_pghalfvec(&b).unwrap(), vec![1.0, -2.0, 0.5]);
        assert!(decode_pghalfvec(&b[..9]).is_err());
    }

    #[test]
    fn test_decode_pgsparsevec() {
        let mut b = Vec::new();
        for v in [5i32, 2, 0, 0, 2] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        for v in [1.5f32, 2.5] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(decode_pgsparsevec(&b).unwrap(), "{1:1.5,3:2.5}/5");
        assert!(decode_pgsparsevec(&b[..b.len() - 1]).is_err());
    }

    #[test]
    fn test_decode_pgsparsevec_empty() {
        let mut b = Vec::new();
        for v in [3i32, 0, 0] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(decode_pgsparsevec(&b).unwrap(), "{}/3");
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
        let connector = PgConnector::new(
            Duration::from_secs(300),
            Duration::from_secs(60),
            100,
            ConnectorConfig::default(),
        );
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
