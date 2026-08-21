use std::sync::Arc;

use arrow::array::{LargeBinaryArray, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use commons::api::errors::ConnectorError;
use commons::api::tabular::QueryOutput;
use futures::TryStreamExt;

use crate::decode::ByteStream;

pub fn binary_schema() -> Schema {
    Schema::new(vec![
        Field::new("data", DataType::LargeBinary, false),
        Field::new("offset", DataType::UInt64, false),
    ])
}

pub async fn read_binary_batches(stream: ByteStream, batch_size: usize) -> QueryOutput {
    if batch_size == 0 {
        return Err(ConnectorError::InvalidRequest(
            "batch_size must be greater than 0".to_string(),
        ));
    }
    let chunk_size = batch_size * 4096;
    let mut stream = stream;
    let output = async_stream::try_stream! {
        let mut offset: u64 = 0;
        let mut buffer = Vec::with_capacity(chunk_size);

        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| ConnectorError::IOError(format!("Binary stream read error: {e}")))?
        {
            buffer.extend_from_slice(&chunk);

            while buffer.len() >= chunk_size {
                let data: Vec<u8> = buffer.drain(..chunk_size).collect();
                let batch = make_binary_batch(&data, offset)?;
                offset += data.len() as u64;
                yield batch;
            }
        }

        if !buffer.is_empty() {
            let batch = make_binary_batch(&buffer, offset)?;
            yield batch;
        }
    };
    Ok(Box::pin(output))
}

fn make_binary_batch(data: &[u8], offset: u64) -> Result<RecordBatch, ConnectorError> {
    let schema = Arc::new(binary_schema());
    let data_array = LargeBinaryArray::from(vec![data]);
    let offset_array = UInt64Array::from(vec![offset]);

    RecordBatch::try_new(schema, vec![Arc::new(data_array), Arc::new(offset_array)])
        .map_err(|e| ConnectorError::IOError(format!("Failed to create binary batch: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_batch() {
        let data = b"hello world";
        let batch = make_binary_batch(data, 42).unwrap();
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 2);

        let col0 = batch.column(0).as_any().downcast_ref::<LargeBinaryArray>().unwrap();
        assert_eq!(col0.value(0), b"hello world");

        let col1 = batch.column(1).as_any().downcast_ref::<UInt64Array>().unwrap();
        assert_eq!(col1.value(0), 42);
    }
}
