use std::io::Cursor;
use std::sync::Arc;

use arrow::datatypes::Schema;
use arrow_csv::ReaderBuilder as CsvReaderBuilder;
use commons::api::errors::ConnectorError;
use commons::api::tabular::QueryOutput;

use crate::decode::{self, ByteStream, Decoder};

pub async fn read_csv_schema(stream: ByteStream) -> Result<Schema, ConnectorError> {
    let buf = decode::read_sample(stream).await?;
    let cursor = Cursor::new(buf);
    let (schema, _) = arrow_csv::reader::Format::default()
        .with_header(true)
        .infer_schema(cursor, None)
        .map_err(|e| ConnectorError::IOError(format!("Failed to infer CSV schema: {e}")))?;
    Ok(schema)
}

pub async fn read_csv_batches(stream: ByteStream, schema: &Arc<Schema>, batch_size: usize) -> QueryOutput {
    if batch_size == 0 {
        return Err(ConnectorError::InvalidRequest(
            "batch_size must be greater than 0".to_string(),
        ));
    }
    let decoder = CsvReaderBuilder::new(schema.clone())
        .with_header(true)
        .with_batch_size(batch_size)
        .build_decoder();

    decode::decode_stream(stream, Decoder::Csv(Box::new(decoder)), "CSV").await
}
