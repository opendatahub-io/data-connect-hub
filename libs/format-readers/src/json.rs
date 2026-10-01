use std::io::{BufWriter, Write};
use std::sync::Arc;

use arrow::datatypes::Schema;
use commons::api::connector::QueryOutput;
use commons::api::errors::ConnectorError;

fn resolve_data_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn extract_rows<'a>(
    response: &'a serde_json::Value,
    data_path: Option<&str>,
) -> Result<&'a Vec<serde_json::Value>, ConnectorError> {
    let target = match data_path {
        Some(path) => resolve_data_path(response, path)
            .ok_or_else(|| ConnectorError::InvalidRequest(format!("data_path '{path}' not found in response")))?,
        None => response,
    };

    target
        .as_array()
        .ok_or_else(|| ConnectorError::InvalidRequest("Response data is not a JSON array".to_string()))
}

type JsonBufReader = arrow_json::Reader<std::io::BufReader<std::io::Cursor<Vec<u8>>>>;

fn build_reader(
    rows: &[serde_json::Value],
    schema: Arc<Schema>,
    batch_size: usize,
) -> Result<JsonBufReader, ConnectorError> {
    let mut buf = BufWriter::new(Vec::new());
    for row in rows {
        serde_json::to_writer(&mut buf, row)
            .map_err(|e| ConnectorError::IOError(format!("Failed to serialize JSON row: {e}")))?;
        buf.write_all(b"\n")
            .map_err(|e| ConnectorError::IOError(format!("Failed to write newline: {e}")))?;
    }
    let jsonl = buf
        .into_inner()
        .map_err(|e| ConnectorError::IOError(format!("Failed to flush buffer: {e}")))?;

    let cursor = std::io::BufReader::new(std::io::Cursor::new(jsonl));
    arrow_json::ReaderBuilder::new(schema)
        .with_batch_size(batch_size)
        .with_coerce_primitive(true)
        .build(cursor)
        .map_err(|e| ConnectorError::IOError(format!("Failed to build JSON reader: {e}")))
}

pub fn read_json_schema(data: &[u8], data_path: Option<&str>) -> Result<Schema, ConnectorError> {
    let json: serde_json::Value =
        serde_json::from_slice(data).map_err(|e| ConnectorError::IOError(format!("Failed to parse JSON: {e}")))?;
    let rows = extract_rows(&json, data_path)?;
    if rows.is_empty() {
        return Err(ConnectorError::NoDataError);
    }
    let schema =
        arrow_json::reader::infer_json_schema_from_iterator(rows.iter().map(Ok::<_, arrow::error::ArrowError>))
            .map_err(|e| ConnectorError::IOError(format!("Failed to infer JSON schema: {e}")))?;
    Ok(schema)
}

pub fn read_json_batches(
    data: Vec<u8>,
    schema: Arc<Schema>,
    batch_size: usize,
    data_path: Option<&str>,
) -> QueryOutput {
    let data_path = data_path.map(String::from);
    let stream = async_stream::try_stream! {
        let json: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|e| ConnectorError::IOError(format!("Failed to parse JSON: {e}")))?;
        let rows = extract_rows(&json, data_path.as_deref())?;

        for chunk in rows.chunks(batch_size) {
            let reader = build_reader(chunk, Arc::clone(&schema), batch_size)?;
            for batch in reader {
                yield batch.map_err(|e| ConnectorError::IOError(format!("JSON decode error: {e}")))?;
            }
        }
    };
    Ok(Box::pin(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
    use arrow::datatypes::{DataType as ArrowDataType, Field};
    use arrow::record_batch::RecordBatch;
    use futures::TryStreamExt;

    #[test]
    fn test_resolve_data_path() {
        let val = serde_json::json!({"results": {"items": [1, 2, 3]}});
        let items = resolve_data_path(&val, "results.items").unwrap();
        assert_eq!(items, &serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn test_resolve_data_path_missing() {
        let val = serde_json::json!({"a": 1});
        assert!(resolve_data_path(&val, "b.c").is_none());
    }

    #[test]
    fn test_extract_rows_top_level_array() {
        let val = serde_json::json!([{"a": 1}, {"a": 2}]);
        let rows = extract_rows(&val, None).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_extract_rows_with_data_path() {
        let val = serde_json::json!({"data": {"rows": [{"x": 1}]}});
        let rows = extract_rows(&val, Some("data.rows")).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_extract_rows_not_array() {
        let val = serde_json::json!({"data": "not an array"});
        assert!(extract_rows(&val, None).is_err());
    }

    #[test]
    fn test_extract_rows_missing_path() {
        let val = serde_json::json!({"a": 1});
        assert!(extract_rows(&val, Some("missing.path")).is_err());
    }

    #[test]
    fn test_read_json_schema_basic() {
        let data = serde_json::to_vec(&serde_json::json!([
            {"name": "Alice", "age": 30, "active": true},
            {"name": "Bob", "age": 25, "active": false},
        ]))
        .unwrap();
        let schema = read_json_schema(&data, None).unwrap();
        assert_eq!(schema.fields().len(), 3);
        assert!(schema.field_with_name("name").is_ok());
        assert!(schema.field_with_name("age").is_ok());
        assert!(schema.field_with_name("active").is_ok());
        assert_eq!(
            *schema.field_with_name("active").unwrap().data_type(),
            ArrowDataType::Boolean
        );
        assert_eq!(
            *schema.field_with_name("age").unwrap().data_type(),
            ArrowDataType::Int64
        );
        assert_eq!(
            *schema.field_with_name("name").unwrap().data_type(),
            ArrowDataType::Utf8
        );
    }

    #[test]
    fn test_read_json_schema_type_conflict_fallback() {
        let data = serde_json::to_vec(&serde_json::json!([
            {"value": 42},
            {"value": "text"},
        ]))
        .unwrap();
        let schema = read_json_schema(&data, None).unwrap();
        assert_eq!(
            *schema.field_with_name("value").unwrap().data_type(),
            ArrowDataType::Utf8
        );
    }

    #[test]
    fn test_read_json_schema_null_values() {
        let data = serde_json::to_vec(&serde_json::json!([
            {"name": null, "age": 30},
            {"name": "Bob", "age": 25},
        ]))
        .unwrap();
        let schema = read_json_schema(&data, None).unwrap();
        assert_eq!(
            *schema.field_with_name("name").unwrap().data_type(),
            ArrowDataType::Utf8
        );
    }

    #[test]
    fn test_read_json_schema_empty() {
        let data = serde_json::to_vec(&serde_json::json!([])).unwrap();
        assert!(read_json_schema(&data, None).is_err());
    }

    #[test]
    fn test_read_json_schema_with_data_path() {
        let data = serde_json::to_vec(&serde_json::json!({"data": {"rows": [{"x": 1}]}})).unwrap();
        let schema = read_json_schema(&data, Some("data.rows")).unwrap();
        assert_eq!(schema.fields().len(), 1);
        assert!(schema.field_with_name("x").is_ok());
    }

    #[tokio::test]
    async fn test_read_json_batches_basic() {
        let data = serde_json::to_vec(&serde_json::json!([
            {"name": "a", "value": 1},
            {"name": "b", "value": 2},
        ]))
        .unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, true),
            Field::new("value", ArrowDataType::Int64, true),
        ]));
        let batches: Vec<RecordBatch> = read_json_batches(data, schema, 1024, None)
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 2);

        let name_arr = batches[0]
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(name_arr.value(0), "a");
        assert_eq!(name_arr.value(1), "b");

        let val_arr = batches[0]
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(val_arr.value(0), 1);
        assert_eq!(val_arr.value(1), 2);
    }

    #[tokio::test]
    async fn test_read_json_batches_with_nulls() {
        let data = serde_json::to_vec(&serde_json::json!([
            {"name": "a"},
            {"name": "b", "value": null},
        ]))
        .unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, true),
            Field::new("value", ArrowDataType::Int64, true),
        ]));
        let batches: Vec<RecordBatch> = read_json_batches(data, schema, 1024, None)
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(batches[0].num_rows(), 2);

        let val_arr = batches[0]
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert!(val_arr.is_null(0));
        assert!(val_arr.is_null(1));
    }

    #[tokio::test]
    async fn test_read_json_batches_with_data_path() {
        let data = serde_json::to_vec(&serde_json::json!({"results": [{"id": 1}, {"id": 2}]})).unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new("id", ArrowDataType::Int64, true)]));
        let batches: Vec<RecordBatch> = read_json_batches(data, schema, 1024, Some("results"))
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(batches[0].num_rows(), 2);
    }

    #[tokio::test]
    async fn test_read_json_batches_batch_size() {
        let rows: Vec<serde_json::Value> = (0..100).map(|i| serde_json::json!({"id": i})).collect();
        let data = serde_json::to_vec(&rows).unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new("id", ArrowDataType::Int64, true)]));
        let batches: Vec<RecordBatch> = read_json_batches(data, schema, 30, None)
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 100);
        assert_eq!(batches.len(), 4);
        assert_eq!(batches[0].num_rows(), 30);
        assert_eq!(batches[3].num_rows(), 10);

        let all_ids: Vec<i64> = batches
            .iter()
            .flat_map(|b| {
                let arr = b
                    .column_by_name("id")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                (0..arr.len()).map(|i| arr.value(i)).collect::<Vec<_>>()
            })
            .collect();
        let expected: Vec<i64> = (0..100).collect();
        assert_eq!(all_ids, expected);
    }

    #[tokio::test]
    async fn test_read_json_batches_multiple_types() {
        let data = serde_json::to_vec(&serde_json::json!([
            {"flag": true, "score": 1.5, "count": 10, "label": "x"},
            {"flag": false, "score": 2.5, "count": 20, "label": "y"},
        ]))
        .unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("flag", ArrowDataType::Boolean, true),
            Field::new("score", ArrowDataType::Float64, true),
            Field::new("count", ArrowDataType::Int64, true),
            Field::new("label", ArrowDataType::Utf8, true),
        ]));
        let batches: Vec<RecordBatch> = read_json_batches(data, schema, 1024, None)
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(batches[0].num_rows(), 2);

        let flags = batches[0]
            .column_by_name("flag")
            .unwrap()
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        assert!(flags.value(0));
        assert!(!flags.value(1));

        let scores = batches[0]
            .column_by_name("score")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((scores.value(0) - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_read_json_schema_field_names() {
        let data = serde_json::to_vec(&serde_json::json!([{"z": 1, "a": 2, "m": 3}])).unwrap();
        let schema = read_json_schema(&data, None).unwrap();
        let mut names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["a", "m", "z"]);
    }

    fn testdata_bytes(filename: &str) -> Vec<u8> {
        let path = format!("{}/testdata/{filename}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(path).unwrap()
    }

    #[test]
    fn test_json_schema_from_file() {
        let data = testdata_bytes("sample.json");
        let schema = read_json_schema(&data, None).unwrap();
        assert_eq!(schema.fields().len(), 4);
        let mut names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["active", "id", "name", "score"]);
    }

    #[tokio::test]
    async fn test_json_batches_from_file() {
        let data = testdata_bytes("sample.json");
        let schema = Arc::new(read_json_schema(&data, None).unwrap());
        let batches: Vec<RecordBatch> = read_json_batches(data, schema, 1024, None)
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 5);
    }

    #[tokio::test]
    async fn test_read_json_batches_mixed_type_coerced_to_string() {
        let data = serde_json::to_vec(&serde_json::json!([
            {"value": 42},
            {"value": "text"},
        ]))
        .unwrap();
        let schema = Arc::new(read_json_schema(&data, None).unwrap());
        assert_eq!(
            *schema.field_with_name("value").unwrap().data_type(),
            ArrowDataType::Utf8
        );

        let batches: Vec<RecordBatch> = read_json_batches(data, schema, 1024, None)
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
}
