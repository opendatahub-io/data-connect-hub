use std::io::Cursor;
use std::sync::Arc;

use arrow::datatypes::Schema;
use commons::api::errors::ConnectorError;
use commons::api::tabular::QueryOutput;
use opendal::Reader;

pub async fn read_jsonl_schema(reader: Reader) -> Result<Schema, ConnectorError> {
    let buf = super::read_sample(reader).await?;
    let cursor = std::io::BufReader::new(Cursor::new(buf));
    let (schema, _) = arrow_json::reader::infer_json_schema(cursor, None)
        .map_err(|e| ConnectorError::IOError(format!("Failed to infer JSONL schema: {e}")))?;
    Ok(schema)
}

pub async fn read_jsonl_batches(reader: Reader, schema: &Arc<Schema>, batch_size: usize) -> QueryOutput {
    let decoder = arrow_json::ReaderBuilder::new(schema.clone())
        .with_batch_size(batch_size)
        .build_decoder()
        .map_err(|e| ConnectorError::IOError(format!("Failed to build JSONL decoder: {e}")))?;

    super::decode_stream(reader, super::Decoder::Json(Box::new(decoder)), "JSONL").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field};
    use futures::TryStreamExt;
    use opendal::{Operator, services::Fs, services::Memory};

    async fn memory_reader(data: &[u8]) -> Reader {
        let op = Operator::new(Memory::default()).unwrap();
        op.write("test.jsonl", data.to_vec()).await.unwrap();
        op.reader("test.jsonl").await.unwrap()
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
    async fn test_jsonl_empty_input() {
        let reader = memory_reader(b"").await;
        let schema = read_jsonl_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 0);
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
