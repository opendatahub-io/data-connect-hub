use std::io::Cursor;
use std::sync::Arc;

use arrow::datatypes::Schema;
use commons::api::errors::ConnectorError;
use commons::api::tabular::QueryOutput;

use crate::decode::{self, ByteStream, Decoder};

pub async fn read_jsonl_schema(stream: ByteStream) -> Result<Schema, ConnectorError> {
    let buf = decode::read_sample(stream).await?;
    let cursor = std::io::BufReader::new(Cursor::new(buf));
    let (schema, _) = arrow_json::reader::infer_json_schema(cursor, None)
        .map_err(|e| ConnectorError::IOError(format!("Failed to infer JSONL schema: {e}")))?;
    Ok(schema)
}

pub async fn read_jsonl_batches(stream: ByteStream, schema: &Arc<Schema>, batch_size: usize) -> QueryOutput {
    if batch_size == 0 {
        return Err(ConnectorError::InvalidRequest(
            "batch_size must be greater than 0".to_string(),
        ));
    }
    let decoder = arrow_json::ReaderBuilder::new(schema.clone())
        .with_batch_size(batch_size)
        .build_decoder()
        .map_err(|e| ConnectorError::IOError(format!("Failed to build JSONL decoder: {e}")))?;

    decode::decode_stream(stream, Decoder::Json(Box::new(decoder)), "JSONL").await
}
