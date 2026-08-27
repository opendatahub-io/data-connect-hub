use crate::api::connections::DataConnectionResource;
use crate::api::errors::ConnectorError;
use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

pub type OutputStream = Pin<Box<dyn Stream<Item = Result<RecordBatch, ConnectorError>> + Send>>;
pub type QueryOutput = Result<OutputStream, ConnectorError>;

#[async_trait::async_trait]
pub trait CredentialsResolver: Send + Sync {
    async fn resolve(&self, connection: &DataConnectionResource) -> Result<HashMap<String, String>, ConnectorError>;
}

pub struct Query {
    pub query: String,
    pub schema: Arc<Schema>,
}

pub struct BinaryQuery {
    pub path: String,
}

impl BinaryQuery {
    pub fn new(path: String) -> Self {
        Self { path }
    }
}

impl Query {
    pub fn new(query: String, schema: Arc<Schema>) -> Self {
        Self { query, schema }
    }
}

pub struct TableInfo {
    pub catalog: String,
    pub schema_name: String,
    pub table_name: String,
    pub table_type: String,
    pub table_schema: Schema,
}

#[derive(Debug, Clone)]
pub struct QueryOptions {
    pub batch_size: usize,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self { batch_size: 512 }
    }
}

#[async_trait::async_trait]
pub trait DataReader: Send + Sync {
    fn provider(&self) -> String;

    async fn schema(&self, query: &str) -> Result<Arc<Query>, ConnectorError>;

    async fn read_tabular(&self, query: Arc<Query>, options: &QueryOptions) -> QueryOutput;

    async fn can_read_binary(&self, _query: Arc<BinaryQuery>) -> Result<(), ConnectorError> {
        Err(ConnectorError::UnsupportedOperation(
            "binary reads are not supported for this connector".to_string(),
        ))
    }

    async fn read_binary(&self, _query: Arc<BinaryQuery>) -> QueryOutput {
        Err(ConnectorError::UnsupportedOperation(
            "binary reads are not supported for this connector".to_string(),
        ))
    }

    async fn check_connection(&self) -> Result<(), ConnectorError>;

    async fn list_tables(
        &self,
        _table_name_filter: Option<&str>,
        _include_schema: bool,
    ) -> Result<Vec<TableInfo>, ConnectorError> {
        Ok(vec![])
    }
}

#[async_trait::async_trait]
pub trait FlightConnector: Send + Sync {
    fn provider(&self) -> String;
    fn description(&self) -> String;

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
        credentials_resolver: &dyn CredentialsResolver,
    ) -> Result<Arc<dyn DataReader>, ConnectorError>;
}

#[cfg(test)]
mod tests {
    use arrow::datatypes::{DataType, Field};

    use super::*;

    #[test]
    fn test_table_info() {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]);
        let info = TableInfo {
            catalog: "mydb".to_string(),
            schema_name: "public".to_string(),
            table_name: "users".to_string(),
            table_type: "TABLE".to_string(),
            table_schema: schema,
        };
        assert_eq!(info.catalog, "mydb");
        assert_eq!(info.schema_name, "public");
        assert_eq!(info.table_name, "users");
        assert_eq!(info.table_type, "TABLE");
        assert_eq!(info.table_schema.fields().len(), 2);
        assert_eq!(info.table_schema.field(0).name(), "id");
        assert_eq!(info.table_schema.field(1).name(), "name");
    }

    #[test]
    fn test_tabular_state_new() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let state = Query::new("SELECT * FROM users".to_string(), schema.clone());

        assert_eq!(state.query, "SELECT * FROM users");
        assert_eq!(state.schema.fields().len(), 2);
        assert_eq!(state.schema.field(0).name(), "id");
        assert_eq!(*state.schema.field(0).data_type(), DataType::Int64);
        assert!(!state.schema.field(0).is_nullable());
        assert_eq!(state.schema.field(1).name(), "name");
        assert_eq!(*state.schema.field(1).data_type(), DataType::Utf8);
        assert!(state.schema.field(1).is_nullable());
    }
}
