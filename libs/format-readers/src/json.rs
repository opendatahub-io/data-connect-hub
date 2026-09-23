use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array, StringArray,
    TimestampMillisecondArray,
};
use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use commons::api::connector::QueryOutput;
use commons::api::errors::ConnectorError;

pub fn infer_arrow_type(value: &serde_json::Value) -> ArrowDataType {
    match value {
        serde_json::Value::Bool(_) => ArrowDataType::Boolean,
        serde_json::Value::Number(n) => {
            if n.is_f64() && n.as_i64().is_none() && n.as_u64().is_none() {
                ArrowDataType::Float64
            } else if n.as_i64().is_none() {
                ArrowDataType::Utf8
            } else {
                ArrowDataType::Int64
            }
        },
        serde_json::Value::String(s) => {
            if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
                ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
            } else {
                ArrowDataType::Utf8
            }
        },
        _ => ArrowDataType::Utf8,
    }
}

pub fn infer_schema(rows: &[serde_json::Value]) -> Schema {
    let mut fields_map: std::collections::BTreeMap<String, ArrowDataType> = std::collections::BTreeMap::new();

    for row in rows {
        if let Some(obj) = row.as_object() {
            for (key, value) in obj {
                if value.is_null() {
                    fields_map.entry(key.clone()).or_insert(ArrowDataType::Utf8);
                    continue;
                }
                let inferred = infer_arrow_type(value);
                fields_map
                    .entry(key.clone())
                    .and_modify(|existing| {
                        if *existing != inferred {
                            *existing = ArrowDataType::Utf8;
                        }
                    })
                    .or_insert(inferred);
            }
        }
    }

    Schema::new(
        fields_map
            .into_iter()
            .map(|(name, dt)| Field::new(name, dt, true))
            .collect::<Vec<_>>(),
    )
}

pub fn resolve_data_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

pub fn extract_rows<'a>(
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

pub fn rows_to_record_batch(schema: &Arc<Schema>, rows: &[serde_json::Value]) -> Result<RecordBatch, ConnectorError> {
    let arrays: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|field| {
            let values: Vec<Option<&serde_json::Value>> = rows
                .iter()
                .map(|row| row.get(field.name()).filter(|v| !v.is_null()))
                .collect();
            json_values_to_array(field.data_type(), &values)
        })
        .collect::<Result<_, _>>()?;

    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(|e| ConnectorError::SQLError(e.to_string()))
}

pub fn json_values_to_array(
    data_type: &ArrowDataType,
    values: &[Option<&serde_json::Value>],
) -> Result<ArrayRef, ConnectorError> {
    match data_type {
        ArrowDataType::Boolean => {
            let arr: BooleanArray = values.iter().map(|v| v.and_then(|v| v.as_bool())).collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int8 => {
            let arr: Int8Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_i64()).map(|n| n as i8))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int16 => {
            let arr: Int16Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_i64()).map(|n| n as i16))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int32 => {
            let arr: Int32Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_i64()).map(|n| n as i32))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int64 => {
            let arr: Int64Array = values.iter().map(|v| v.and_then(|v| v.as_i64())).collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Float32 => {
            let arr: Float32Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_f64()).map(|n| n as f32))
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Float64 => {
            let arr: Float64Array = values.iter().map(|v| v.and_then(|v| v.as_f64())).collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Timestamp(TimeUnit::Millisecond, _) => {
            let arr: TimestampMillisecondArray = values
                .iter()
                .map(|v| {
                    v.and_then(|v| {
                        v.as_i64().or_else(|| {
                            v.as_str().and_then(|s| {
                                chrono::DateTime::parse_from_rfc3339(s)
                                    .ok()
                                    .map(|dt| dt.timestamp_millis())
                                    .or_else(|| {
                                        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                                            .ok()
                                            .map(|dt| dt.and_utc().timestamp_millis())
                                    })
                            })
                        })
                    })
                })
                .collect();
            Ok(Arc::new(arr.with_timezone("UTC")))
        },
        _ => {
            let arr: StringArray = values
                .iter()
                .map(|v| {
                    v.map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                })
                .collect();
            Ok(Arc::new(arr))
        },
    }
}

pub fn read_json_schema(data: &[u8], data_path: Option<&str>) -> Result<Schema, ConnectorError> {
    let json: serde_json::Value =
        serde_json::from_slice(data).map_err(|e| ConnectorError::IOError(format!("Failed to parse JSON: {e}")))?;
    let rows = extract_rows(&json, data_path)?;
    if rows.is_empty() {
        return Err(ConnectorError::NoDataError);
    }
    Ok(infer_schema(rows))
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
            let batch = rows_to_record_batch(&schema, chunk)?;
            yield batch;
        }
    };
    Ok(Box::pin(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;

    #[test]
    fn test_infer_arrow_type_bool() {
        assert_eq!(infer_arrow_type(&serde_json::json!(true)), ArrowDataType::Boolean);
    }

    #[test]
    fn test_infer_arrow_type_int() {
        assert_eq!(infer_arrow_type(&serde_json::json!(42)), ArrowDataType::Int64);
    }

    #[test]
    fn test_infer_arrow_type_u64_above_i64_max() {
        let val = serde_json::json!(18_446_744_073_709_551_615_u64);
        assert_eq!(infer_arrow_type(&val), ArrowDataType::Utf8);
    }

    #[test]
    fn test_infer_arrow_type_float() {
        assert_eq!(infer_arrow_type(&serde_json::json!(1.23)), ArrowDataType::Float64);
    }

    #[test]
    fn test_infer_arrow_type_string() {
        assert_eq!(infer_arrow_type(&serde_json::json!("hello")), ArrowDataType::Utf8);
    }

    #[test]
    fn test_infer_arrow_type_timestamp() {
        assert_eq!(
            infer_arrow_type(&serde_json::json!("2023-11-14T22:13:20.000Z")),
            ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );
    }

    #[test]
    fn test_infer_arrow_type_null() {
        assert_eq!(infer_arrow_type(&serde_json::Value::Null), ArrowDataType::Utf8);
    }

    #[test]
    fn test_infer_schema_basic() {
        let rows = vec![
            serde_json::json!({"name": "Alice", "age": 30, "active": true}),
            serde_json::json!({"name": "Bob", "age": 25, "active": false}),
        ];
        let schema = infer_schema(&rows);
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "active");
        assert_eq!(*schema.field(0).data_type(), ArrowDataType::Boolean);
        assert_eq!(schema.field(1).name(), "age");
        assert_eq!(*schema.field(1).data_type(), ArrowDataType::Int64);
        assert_eq!(schema.field(2).name(), "name");
        assert_eq!(*schema.field(2).data_type(), ArrowDataType::Utf8);
    }

    #[test]
    fn test_infer_schema_sorted_alphabetically() {
        let rows = vec![serde_json::json!({"z": 1, "a": 2, "m": 3})];
        let schema = infer_schema(&rows);
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec!["a", "m", "z"]);
    }

    #[test]
    fn test_infer_schema_type_conflict_fallback() {
        let rows = vec![serde_json::json!({"value": 42}), serde_json::json!({"value": "text"})];
        let schema = infer_schema(&rows);
        assert_eq!(*schema.field(0).data_type(), ArrowDataType::Utf8);
    }

    #[test]
    fn test_infer_schema_null_values() {
        let rows = vec![
            serde_json::json!({"name": null, "age": 30}),
            serde_json::json!({"name": "Bob", "age": 25}),
        ];
        let schema = infer_schema(&rows);
        assert_eq!(*schema.field(1).data_type(), ArrowDataType::Utf8);
    }

    #[test]
    fn test_infer_schema_empty() {
        let schema = infer_schema(&[]);
        assert_eq!(schema.fields().len(), 0);
    }

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
    fn test_json_values_to_array_boolean() {
        let v_true = serde_json::json!(true);
        let v_false = serde_json::json!(false);
        let vals = vec![Some(&v_true), None, Some(&v_false)];
        let arr = json_values_to_array(&ArrowDataType::Boolean, &vals).unwrap();
        let bool_arr = arr.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert_eq!(bool_arr.len(), 3);
        assert!(bool_arr.value(0));
        assert!(bool_arr.is_null(1));
        assert!(!bool_arr.value(2));
    }

    #[test]
    fn test_json_values_to_array_int64() {
        let v1 = serde_json::json!(42);
        let v2 = serde_json::json!(99);
        let vals = vec![Some(&v1), Some(&v2), None];
        let arr = json_values_to_array(&ArrowDataType::Int64, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(int_arr.value(0), 42);
        assert_eq!(int_arr.value(1), 99);
        assert!(int_arr.is_null(2));
    }

    #[test]
    fn test_json_values_to_array_float64() {
        let v = serde_json::json!(1.23);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array(&ArrowDataType::Float64, &vals).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((f_arr.value(0) - 1.23).abs() < f64::EPSILON);
        assert!(f_arr.is_null(1));
    }

    #[test]
    fn test_json_values_to_array_utf8_fallback() {
        let v_str = serde_json::json!("hello");
        let v_obj = serde_json::json!({"nested": true});
        let vals = vec![Some(&v_str), Some(&v_obj), None];
        let arr = json_values_to_array(&ArrowDataType::Utf8, &vals).unwrap();
        let str_arr = arr.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(str_arr.value(0), "hello");
        assert_eq!(str_arr.value(1), r#"{"nested":true}"#);
        assert!(str_arr.is_null(2));
    }

    #[test]
    fn test_json_values_to_array_timestamp_epoch() {
        let v = serde_json::json!(1700000000000_i64);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array(
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
        assert!(ts_arr.is_null(1));
    }

    #[test]
    fn test_json_values_to_array_timestamp_iso() {
        let v = serde_json::json!("2023-11-14T22:13:20.000Z");
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
    }

    #[test]
    fn test_rows_to_record_batch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, true),
            Field::new("value", ArrowDataType::Int64, true),
        ]));
        let rows = vec![
            serde_json::json!({"name": "a", "value": 1}),
            serde_json::json!({"name": "b", "value": 2}),
        ];
        let batch = rows_to_record_batch(&schema, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);

        let name_arr = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(name_arr.value(0), "a");
        assert_eq!(name_arr.value(1), "b");

        let val_arr = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(val_arr.value(0), 1);
        assert_eq!(val_arr.value(1), 2);
    }

    #[test]
    fn test_rows_to_record_batch_with_nulls() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", ArrowDataType::Utf8, true),
            Field::new("value", ArrowDataType::Int64, true),
        ]));
        let rows = vec![
            serde_json::json!({"name": "a"}),
            serde_json::json!({"name": "b", "value": null}),
        ];
        let batch = rows_to_record_batch(&schema, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);

        let val_arr = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        assert!(val_arr.is_null(0));
        assert!(val_arr.is_null(1));
    }
}
