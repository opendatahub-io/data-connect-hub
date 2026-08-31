use std::io::Cursor;
use std::sync::Arc;

use arrow::datatypes::Schema;
use arrow_csv::ReaderBuilder as CsvReaderBuilder;
use commons::api::connector::QueryOutput;
use commons::api::errors::ConnectorError;
use opendal::Reader;

pub async fn read_csv_schema(reader: Reader) -> Result<Schema, ConnectorError> {
    let buf = super::read_sample(reader).await?;
    let cursor = Cursor::new(buf);
    let (schema, _) = arrow_csv::reader::Format::default()
        .with_header(true)
        .infer_schema(cursor, None)
        .map_err(|e| ConnectorError::IOError(format!("Failed to infer CSV schema: {e}")))?;
    Ok(schema)
}

pub async fn read_csv_batches(reader: Reader, schema: &Arc<Schema>, batch_size: usize) -> QueryOutput {
    let decoder = CsvReaderBuilder::new(schema.clone())
        .with_header(true)
        .with_batch_size(batch_size)
        .build_decoder();

    super::decode_stream(reader, super::Decoder::Csv(Box::new(decoder)), "CSV").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field};
    use futures::TryStreamExt;
    use opendal::{Operator, services::Fs, services::Memory};

    async fn memory_reader(data: &[u8]) -> Reader {
        let op = Operator::new(Memory::default()).unwrap();
        op.write("test.csv", data.to_vec()).await.unwrap();
        op.reader("test.csv").await.unwrap()
    }

    #[tokio::test]
    async fn test_csv_roundtrip() {
        let csv_data = b"id,name,score\n1,alice,95.5\n2,bob,87.0\n3,charlie,92.3\n";

        let reader = memory_reader(csv_data).await;
        let schema = read_csv_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 3);

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, true),
            Field::new("name", DataType::Utf8, true),
            Field::new("score", DataType::Float64, true),
        ]));

        let reader = memory_reader(csv_data).await;
        let batches: Vec<_> = read_csv_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 3);

        let ids = batches[0].column(0).as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(ids.value(0), 1);
        assert_eq!(ids.value(1), 2);

        let names = batches[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(names.value(0), "alice");

        let scores = batches[0].column(2).as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((scores.value(0) - 95.5).abs() < f64::EPSILON);
    }

    fn testdata_reader(filename: &str) -> Reader {
        let testdata_dir = format!("{}/testdata", env!("CARGO_MANIFEST_DIR"));
        let op = Operator::new(Fs::default().root(&testdata_dir)).unwrap();
        futures::executor::block_on(op.reader(filename)).unwrap()
    }

    #[tokio::test]
    async fn test_csv_schema_from_file() {
        let reader = testdata_reader("sample.csv");
        let schema = read_csv_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 4);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(schema.field(2).name(), "score");
        assert_eq!(schema.field(3).name(), "active");
    }

    #[tokio::test]
    async fn test_csv_batches_from_file() {
        let reader = testdata_reader("sample.csv");
        let schema = read_csv_schema(reader).await.unwrap();

        let schema = Arc::new(schema);
        let reader = testdata_reader("sample.csv");
        let batches: Vec<_> = read_csv_batches(reader, &schema, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 5);
    }
}
