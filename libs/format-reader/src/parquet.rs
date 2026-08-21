use arrow::datatypes::Schema;
use commons::api::errors::ConnectorError;
use commons::api::tabular::QueryOutput;
use futures::StreamExt;
use parquet::arrow::async_reader::{AsyncFileReader, ParquetRecordBatchStreamBuilder};

pub async fn read_parquet_schema(reader: impl AsyncFileReader + Unpin + 'static) -> Result<Schema, ConnectorError> {
    let builder = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to read Parquet metadata: {e}")))?;
    Ok(builder.schema().as_ref().clone())
}

pub async fn read_parquet_batches(reader: impl AsyncFileReader + Unpin + 'static, batch_size: usize) -> QueryOutput {
    let stream = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to open Parquet reader: {e}")))?
        .with_batch_size(batch_size)
        .build()
        .map_err(|e| ConnectorError::IOError(format!("Failed to build Parquet reader: {e}")))?;

    Ok(Box::pin(stream.map(|batch| {
        batch.map_err(|e| ConnectorError::IOError(format!("Parquet read error: {e}")))
    })))
}
