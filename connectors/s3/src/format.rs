use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;
use arrow_csv::ReaderBuilder as CsvReaderBuilder;
use bytes::Bytes;
use commons::api::errors::ConnectorError;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::io::Cursor;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileFormat {
    Parquet,
    Csv,
}

impl FileFormat {
    pub fn detect(path: &str, properties: Option<&str>) -> Result<Self, ConnectorError> {
        if let Some(fmt) = properties {
            return match fmt.to_lowercase().as_str() {
                "parquet" => Ok(FileFormat::Parquet),
                "csv" => Ok(FileFormat::Csv),
                other => Err(ConnectorError::InvalidRequest(format!("Unsupported format: {other}"))),
            };
        }

        let lower = path.to_lowercase();
        if lower.ends_with(".parquet") {
            Ok(FileFormat::Parquet)
        } else if lower.ends_with(".csv") {
            Ok(FileFormat::Csv)
        } else {
            Err(ConnectorError::InvalidRequest(format!(
                "Cannot detect format for path: {path}. Set 'format' in connection properties."
            )))
        }
    }
}

pub fn read_parquet_schema(data: &Bytes) -> Result<Schema, ConnectorError> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(data.clone())
        .map_err(|e| ConnectorError::IOError(format!("Failed to read Parquet metadata: {e}")))?;
    Ok(reader.schema().as_ref().clone())
}

pub fn read_parquet_batches(data: Bytes, batch_size: usize) -> Result<Vec<RecordBatch>, ConnectorError> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(data)
        .map_err(|e| ConnectorError::IOError(format!("Failed to open Parquet reader: {e}")))?
        .with_batch_size(batch_size)
        .build()
        .map_err(|e| ConnectorError::IOError(format!("Failed to build Parquet reader: {e}")))?;

    reader
        .into_iter()
        .map(|batch| batch.map_err(|e| ConnectorError::IOError(format!("Parquet read error: {e}"))))
        .collect()
}

pub fn read_csv_schema(data: &Bytes) -> Result<Schema, ConnectorError> {
    let cursor = Cursor::new(data.as_ref());
    let (schema, _) = arrow_csv::reader::Format::default()
        .with_header(true)
        .infer_schema(cursor, None)
        .map_err(|e| ConnectorError::IOError(format!("Failed to infer CSV schema: {e}")))?;
    Ok(schema)
}

pub fn read_csv_batches(
    data: Bytes,
    schema: &Arc<Schema>,
    batch_size: usize,
) -> Result<Vec<RecordBatch>, ConnectorError> {
    let cursor = Cursor::new(data.as_ref());
    let reader = CsvReaderBuilder::new(schema.clone())
        .with_header(true)
        .with_batch_size(batch_size)
        .build(cursor)
        .map_err(|e| ConnectorError::IOError(format!("Failed to build CSV reader: {e}")))?;

    reader
        .into_iter()
        .map(|batch| batch.map_err(|e| ConnectorError::IOError(format!("CSV read error: {e}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field};

    #[test]
    fn test_detect_format_from_properties() {
        assert_eq!(
            FileFormat::detect("anything", Some("parquet")).unwrap(),
            FileFormat::Parquet
        );
        assert_eq!(FileFormat::detect("anything", Some("csv")).unwrap(), FileFormat::Csv);
        assert_eq!(
            FileFormat::detect("anything", Some("Parquet")).unwrap(),
            FileFormat::Parquet
        );
    }

    #[test]
    fn test_detect_format_from_extension() {
        assert_eq!(
            FileFormat::detect("data/train.parquet", None).unwrap(),
            FileFormat::Parquet
        );
        assert_eq!(FileFormat::detect("data/train.csv", None).unwrap(), FileFormat::Csv);
        assert_eq!(
            FileFormat::detect("data/train.PARQUET", None).unwrap(),
            FileFormat::Parquet
        );
    }

    #[test]
    fn test_detect_format_unknown() {
        let result = FileFormat::detect("data/train.json", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_format_unsupported_property() {
        let result = FileFormat::detect("anything", Some("json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parquet_roundtrip() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec![Some("a"), Some("b"), None])),
            ],
        )
        .unwrap();

        let mut buf = Vec::new();
        {
            let mut writer = parquet::arrow::ArrowWriter::try_new(&mut buf, schema.clone(), None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        let data = Bytes::from(buf);

        let read_schema = read_parquet_schema(&data).unwrap();
        assert_eq!(read_schema.fields().len(), 2);
        assert_eq!(read_schema.field(0).name(), "id");
        assert_eq!(read_schema.field(1).name(), "name");

        let batches = read_parquet_batches(data, 1024).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 3);
    }

    #[test]
    fn test_csv_roundtrip() {
        let csv_data = Bytes::from("id,name,score\n1,alice,95.5\n2,bob,87.0\n3,charlie,92.3\n");

        let schema = read_csv_schema(&csv_data).unwrap();
        assert_eq!(schema.fields().len(), 3);

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, true),
            Field::new("name", DataType::Utf8, true),
            Field::new("score", DataType::Float64, true),
        ]));

        let batches = read_csv_batches(csv_data, &schema, 1024).unwrap();
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

    #[test]
    fn test_parquet_batch_size() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));

        let ids: Vec<i32> = (0..100).collect();
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int32Array::from(ids))]).unwrap();

        let mut buf = Vec::new();
        {
            let mut writer = parquet::arrow::ArrowWriter::try_new(&mut buf, schema.clone(), None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        let data = Bytes::from(buf);

        let batches = read_parquet_batches(data, 30).unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 100);
        assert!(batches.len() >= 3);
    }
}
