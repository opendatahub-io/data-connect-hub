use crate::api::connections::DataConnectionResource;
use crate::api::errors::ConnectorError;
use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;

pub type OutputStream = Pin<Box<dyn Stream<Item = Result<RecordBatch, ConnectorError>> + Send>>;
pub type QueryOutput = Result<OutputStream, ConnectorError>;

pub struct TabularState {
    pub query: String,
    pub schema: Arc<Schema>,
}

impl TabularState {
    pub fn new(query: String, schema: Arc<Schema>) -> Self {
        Self { query, schema }
    }
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
pub trait TabularReader: Send + Sync {
    fn provider(&self) -> String;

    async fn schema(&self, query: &str) -> Result<Arc<TabularState>, ConnectorError>;

    async fn read(&self, state: Arc<TabularState>, options: &QueryOptions) -> QueryOutput;

    async fn test_connection(&self) -> Result<(), ConnectorError>;
}

#[async_trait::async_trait]
pub trait FlightConnector: Send + Sync {
    fn provider(&self) -> String;
    fn description(&self) -> String;

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
    ) -> Result<Arc<dyn TabularReader>, ConnectorError>;
}

#[cfg(test)]
mod tests {
    use arrow::datatypes::{DataType, Field};

    use super::*;

    #[test]
    fn test_tabular_state_new() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let state = TabularState::new("SELECT * FROM users".to_string(), schema.clone());

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
