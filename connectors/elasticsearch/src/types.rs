use std::collections::HashMap;

use arrow::datatypes::{DataType as ArrowDataType, Field, Schema, TimeUnit};

pub fn es_type_to_arrow(es_type: &str) -> ArrowDataType {
    match es_type {
        "boolean" => ArrowDataType::Boolean,
        "byte" => ArrowDataType::Int8,
        "short" => ArrowDataType::Int16,
        "integer" => ArrowDataType::Int32,
        "long" => ArrowDataType::Int64,
        "half_float" | "float" => ArrowDataType::Float32,
        "double" | "scaled_float" => ArrowDataType::Float64,
        "date" | "date_nanos" => ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
        _ => ArrowDataType::Utf8,
    }
}

pub struct EsMappingField {
    pub path: String,
    pub es_type: String,
}

/// Extract fields from an ES _mapping response.
/// Merges fields across all indices (for alias / wildcard queries).
/// When the same field has different types across indices, falls back to "text" (Utf8).
pub fn parse_mapping(mapping_json: &serde_json::Value) -> Vec<EsMappingField> {
    let mut fields = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();

    if let Some(obj) = mapping_json.as_object() {
        let has_index_keys = obj.values().any(|v| v.get("mappings").is_some());
        if has_index_keys {
            for idx_mapping in obj.values() {
                if let Some(props) = idx_mapping.get("mappings").and_then(|m| m.get("properties")) {
                    merge_properties(props, "", &mut fields, &mut seen);
                }
            }
        }
    }

    if fields.is_empty()
        && let Some(props) = mapping_json.get("mappings").and_then(|m| m.get("properties"))
    {
        merge_properties(props, "", &mut fields, &mut seen);
    }

    fields
}

fn merge_properties(
    properties: &serde_json::Value,
    prefix: &str,
    fields: &mut Vec<EsMappingField>,
    seen: &mut HashMap<String, usize>,
) {
    let Some(obj) = properties.as_object() else {
        return;
    };
    for (name, value) in obj {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };

        if let Some(sub_props) = value.get("properties") {
            merge_properties(sub_props, &path, fields, seen);
        } else if let Some(es_type) = value.get("type").and_then(|v| v.as_str()) {
            match seen.get(&path) {
                None => {
                    seen.insert(path.clone(), fields.len());
                    fields.push(EsMappingField {
                        path,
                        es_type: es_type.to_string(),
                    });
                },
                Some(&pos) => {
                    if fields[pos].es_type != es_type {
                        fields[pos].es_type = "text".to_string();
                    }
                },
            }
        }
    }
}

pub fn mapping_fields_to_schema(fields: &[EsMappingField]) -> Schema {
    Schema::new(
        fields
            .iter()
            .map(|f| Field::new(&f.path, es_type_to_arrow(&f.es_type), true))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_es_type_to_arrow() {
        assert_eq!(es_type_to_arrow("boolean"), ArrowDataType::Boolean);
        assert_eq!(es_type_to_arrow("byte"), ArrowDataType::Int8);
        assert_eq!(es_type_to_arrow("short"), ArrowDataType::Int16);
        assert_eq!(es_type_to_arrow("integer"), ArrowDataType::Int32);
        assert_eq!(es_type_to_arrow("long"), ArrowDataType::Int64);
        assert_eq!(es_type_to_arrow("float"), ArrowDataType::Float32);
        assert_eq!(es_type_to_arrow("half_float"), ArrowDataType::Float32);
        assert_eq!(es_type_to_arrow("double"), ArrowDataType::Float64);
        assert_eq!(es_type_to_arrow("scaled_float"), ArrowDataType::Float64);
        assert_eq!(
            es_type_to_arrow("date"),
            ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );
        assert_eq!(es_type_to_arrow("keyword"), ArrowDataType::Utf8);
        assert_eq!(es_type_to_arrow("text"), ArrowDataType::Utf8);
        assert_eq!(es_type_to_arrow("unknown_type"), ArrowDataType::Utf8);
    }

    #[test]
    fn test_parse_mapping_single_index() {
        let mapping = serde_json::json!({
            "my-index": {
                "mappings": {
                    "properties": {
                        "title": { "type": "text" },
                        "count": { "type": "integer" },
                        "active": { "type": "boolean" }
                    }
                }
            }
        });
        let fields = parse_mapping(&mapping);
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn test_parse_mapping_nested() {
        let mapping = serde_json::json!({
            "my-index": {
                "mappings": {
                    "properties": {
                        "user": {
                            "properties": {
                                "name": { "type": "keyword" },
                                "age": { "type": "integer" }
                            }
                        },
                        "title": { "type": "text" }
                    }
                }
            }
        });
        let fields = parse_mapping(&mapping);
        assert_eq!(fields.len(), 3);
        assert!(fields.iter().any(|f| f.path == "user.name"));
        assert!(fields.iter().any(|f| f.path == "user.age"));
        assert!(fields.iter().any(|f| f.path == "title"));
    }

    #[test]
    fn test_parse_mapping_alias() {
        let mapping = serde_json::json!({
            "actual-index-2024": {
                "mappings": {
                    "properties": {
                        "title": { "type": "text" },
                        "count": { "type": "integer" }
                    }
                }
            }
        });
        let fields = parse_mapping(&mapping);
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn test_parse_mapping_multi_index_merges() {
        let mapping = serde_json::json!({
            "index-a": {
                "mappings": {
                    "properties": {
                        "name": { "type": "keyword" },
                        "value": { "type": "integer" }
                    }
                }
            },
            "index-b": {
                "mappings": {
                    "properties": {
                        "name": { "type": "keyword" },
                        "score": { "type": "float" }
                    }
                }
            }
        });
        let fields = parse_mapping(&mapping);
        assert_eq!(fields.len(), 3);
        assert!(fields.iter().any(|f| f.path == "name"));
        assert!(fields.iter().any(|f| f.path == "value"));
        assert!(fields.iter().any(|f| f.path == "score"));
    }

    #[test]
    fn test_parse_mapping_type_conflict() {
        let mapping = serde_json::json!({
            "index-a": {
                "mappings": {
                    "properties": {
                        "status": { "type": "integer" },
                        "name": { "type": "keyword" }
                    }
                }
            },
            "index-b": {
                "mappings": {
                    "properties": {
                        "status": { "type": "keyword" },
                        "name": { "type": "keyword" }
                    }
                }
            }
        });
        let fields = parse_mapping(&mapping);
        assert_eq!(fields.len(), 2);
        let status = fields.iter().find(|f| f.path == "status").unwrap();
        assert_eq!(status.es_type, "text");
        let name = fields.iter().find(|f| f.path == "name").unwrap();
        assert_eq!(name.es_type, "keyword");
    }

    #[test]
    fn test_parse_mapping_empty() {
        let mapping = serde_json::json!({
            "my-index": {
                "mappings": {
                    "properties": {}
                }
            }
        });
        let fields = parse_mapping(&mapping);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_mapping_fields_to_schema() {
        let fields = vec![
            EsMappingField {
                path: "title".to_string(),
                es_type: "text".to_string(),
            },
            EsMappingField {
                path: "count".to_string(),
                es_type: "integer".to_string(),
            },
        ];
        let schema = mapping_fields_to_schema(&fields);
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.field(0).name(), "title");
        assert_eq!(*schema.field(0).data_type(), ArrowDataType::Utf8);
        assert!(schema.field(0).is_nullable());
        assert_eq!(schema.field(1).name(), "count");
        assert_eq!(*schema.field(1).data_type(), ArrowDataType::Int32);
    }
}
