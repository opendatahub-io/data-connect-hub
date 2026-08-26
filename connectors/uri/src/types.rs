use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, TimeUnit};

// JSON Number doesn't distinguish Int8/16/32 or Float32 — we infer the
// widest safe types (Int64/Float64).  The downstream json_values_to_array
// converter handles narrower Arrow types when a caller supplies a schema.
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
) -> Result<&'a Vec<serde_json::Value>, commons::api::errors::ConnectorError> {
    let target = match data_path {
        Some(path) => resolve_data_path(response, path).ok_or_else(|| {
            commons::api::errors::ConnectorError::InvalidRequest(format!("data_path '{path}' not found in response"))
        })?,
        None => response,
    };

    target.as_array().ok_or_else(|| {
        commons::api::errors::ConnectorError::InvalidRequest("Response data is not a JSON array".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
