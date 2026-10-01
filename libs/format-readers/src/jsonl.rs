use std::io::Cursor;
use std::sync::Arc;

use arrow::datatypes::Schema;
use commons::api::connector::QueryOutput;
use commons::api::errors::ConnectorError;
use opendal::Reader;

pub async fn read_jsonl_schema(reader: Reader) -> Result<Schema, ConnectorError> {
    let buf = super::read_sample(reader).await?;
    if buf.iter().all(|&b| b.is_ascii_whitespace()) {
        return Err(ConnectorError::NoDataError);
    }
    let cursor = std::io::BufReader::new(Cursor::new(buf));
    let (schema, _) = arrow_json::reader::infer_json_schema(cursor, None)
        .map_err(|e| ConnectorError::IOError(format!("Failed to infer JSONL schema: {e}")))?;
    Ok(schema)
}

pub async fn read_jsonl_batches(reader: Reader, schema: &Arc<Schema>, batch_size: usize) -> QueryOutput {
    let decoder = arrow_json::ReaderBuilder::new(schema.clone())
        .with_batch_size(batch_size)
        .with_coerce_primitive(true)
        .build_decoder()
        .map_err(|e| ConnectorError::IOError(format!("Failed to build JSONL decoder: {e}")))?;

    super::decode_stream(reader, super::Decoder::Json(Box::new(decoder)), "JSONL").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field};
    use futures::TryStreamExt;
    use opendal::{Operator, services::Fs, services::Memory};

    async fn memory_reader(data: &[u8]) -> Reader {
        let op = Operator::new(Memory::default()).unwrap();
        op.write("test.jsonl", data.to_vec()).await.unwrap();
        op.reader("test.jsonl").await.unwrap()
    }

    async fn chunked_reader(data: &[u8], chunk_size: usize) -> Reader {
        let op = Operator::new(Memory::default()).unwrap();
        op.write("test.jsonl", data.to_vec()).await.unwrap();
        op.reader_with("test.jsonl").chunk(chunk_size).await.unwrap()
    }

    #[tokio::test]
    async fn test_jsonl_roundtrip() {
        let jsonl_data =
            b"{\"id\":1,\"name\":\"alice\",\"score\":95.5}\n{\"id\":2,\"name\":\"bob\",\"score\":87.0}\n{\"id\":3,\"name\":\"charlie\",\"score\":92.3}\n";

        let reader = memory_reader(jsonl_data).await;
        let schema = read_jsonl_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 3);

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
            Field::new("score", DataType::Float64, true),
        ]));

        let reader = memory_reader(jsonl_data).await;
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 3);

        let names = batches[0]
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(names.value(0), "alice");
        assert_eq!(names.value(1), "bob");
        assert_eq!(names.value(2), "charlie");

        let scores = batches[0]
            .column_by_name("score")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((scores.value(0) - 95.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_jsonl_record_split_across_chunks() {
        // Each JSON object is ~20 bytes. chunk_size=10 forces records to
        // be split across multiple chunks.
        let jsonl_data = b"{\"id\":1,\"name\":\"alice\"}\n{\"id\":2,\"name\":\"bob\"}\n";

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ]));

        let reader = chunked_reader(jsonl_data, 10).await;
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        let last_batch = batches.last().unwrap();
        let names = last_batch
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(names.value(names.len() - 1), "bob");
    }

    #[tokio::test]
    async fn test_jsonl_batch_size() {
        let lines: String = (0..100).map(|i| format!("{{\"id\":{i}}}\n")).collect();

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));

        let reader = memory_reader(lines.as_bytes()).await;
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 30)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 100);
        assert!(batches.len() >= 3);
    }

    #[tokio::test]
    async fn test_jsonl_malformed_line() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let reader = memory_reader(b"{\"id\":1}\nnot valid json\n{\"id\":3}\n").await;
        let result: Result<Vec<_>, _> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_jsonl_malformed_line_no_trailing_newline() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let reader = memory_reader(b"{\"id\":1}\nnot valid json").await;
        let result: Result<Vec<_>, _> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_jsonl_empty_input() {
        let reader = memory_reader(b"").await;
        let result = read_jsonl_schema(reader).await;
        assert!(matches!(result, Err(ConnectorError::NoDataError)));
    }

    #[tokio::test]
    async fn test_jsonl_type_conflict() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let reader = memory_reader(b"{\"id\":1}\n{\"id\":\"not_a_number\"}\n").await;
        let result: Result<Vec<_>, _> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_jsonl_mixed_type_coerced_to_string() {
        let jsonl = b"{\"value\":42}\n{\"value\":\"text\"}\n";
        let reader = memory_reader(jsonl).await;
        let schema = read_jsonl_schema(reader).await.unwrap();
        assert_eq!(*schema.field_with_name("value").unwrap().data_type(), DataType::Utf8);

        let schema = Arc::new(schema);
        let reader = memory_reader(jsonl).await;
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(batches[0].num_rows(), 2);

        let arr = batches[0]
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(arr.value(0), "42");
        assert_eq!(arr.value(1), "text");
    }

    #[tokio::test]
    async fn test_jsonl_schema_no_trailing_newline() {
        // The last row (no trailing newline) contains a string value for
        // "value", which forces the inferred type to Utf8 instead of Int64.
        // If read_sample dropped the last row, value would be inferred as Int64.
        let jsonl_data = b"{\"value\":1}\n{\"value\":\"text\"}";
        let reader = memory_reader(jsonl_data).await;
        let schema = read_jsonl_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 1);
        assert_eq!(*schema.field_with_name("value").unwrap().data_type(), DataType::Utf8);
    }

    #[tokio::test]
    async fn test_jsonl_no_trailing_newline() {
        let jsonl_data = b"{\"id\":1,\"name\":\"alice\"}\n{\"id\":2,\"name\":\"bob\"}";

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ]));

        let reader = memory_reader(jsonl_data).await;
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        let last_batch = batches.last().unwrap();
        let names = last_batch
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(names.value(names.len() - 1), "bob");
    }

    #[tokio::test]
    async fn test_jsonl_single_row_no_trailing_newline() {
        let jsonl_data = b"{\"id\":1,\"name\":\"alice\"}";

        let reader = memory_reader(jsonl_data).await;
        let schema = read_jsonl_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 2);

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
        ]));
        let reader = memory_reader(jsonl_data).await;
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 1);

        let names = batches[0]
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(names.value(0), "alice");
    }

    #[tokio::test]
    async fn test_jsonl_empty_input_batches() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let reader = memory_reader(b"").await;
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 0);
    }

    #[tokio::test]
    async fn test_jsonl_only_newlines() {
        let reader = memory_reader(b"\n\n").await;
        let result = read_jsonl_schema(reader).await;
        assert!(result.is_err());
    }

    fn testdata_reader(filename: &str) -> Reader {
        let testdata_dir = format!("{}/testdata", env!("CARGO_MANIFEST_DIR"));
        let op = Operator::new(Fs::default().root(&testdata_dir)).unwrap();
        futures::executor::block_on(op.reader(filename)).unwrap()
    }

    #[tokio::test]
    async fn test_jsonl_schema_from_file() {
        let reader = testdata_reader("sample.jsonl");
        let schema = read_jsonl_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 4);
    }

    #[tokio::test]
    async fn test_jsonl_batches_from_file() {
        let reader = testdata_reader("sample.jsonl");
        let schema = read_jsonl_schema(reader).await.unwrap();

        let schema = Arc::new(schema);
        let reader = testdata_reader("sample.jsonl");
        let batches: Vec<_> = read_jsonl_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 5);
    }
}
