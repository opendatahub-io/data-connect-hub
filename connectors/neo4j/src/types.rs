use arrow::datatypes::{DataType as ArrowDataType, TimeUnit};
use neo4rs::BoltType;

pub fn bolt_type_to_arrow(bolt: &BoltType) -> ArrowDataType {
    match bolt {
        BoltType::Boolean(_) => ArrowDataType::Boolean,
        BoltType::Integer(_) => ArrowDataType::Int64,
        BoltType::Float(_) => ArrowDataType::Float64,
        BoltType::String(_) => ArrowDataType::Utf8,
        BoltType::Bytes(_) => ArrowDataType::Binary,
        BoltType::Date(_) => ArrowDataType::Date32,
        BoltType::DateTime(_) | BoltType::DateTimeZoneId(_) => {
            ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        },
        BoltType::LocalDateTime(_) => ArrowDataType::Timestamp(TimeUnit::Millisecond, None),
        _ => ArrowDataType::Utf8,
    }
}

pub fn bolt_value_to_json(bolt: &BoltType) -> serde_json::Value {
    match bolt {
        BoltType::Null(_) => serde_json::Value::Null,
        BoltType::Boolean(b) => serde_json::Value::Bool(b.value),
        BoltType::Integer(i) => serde_json::json!(i.value),
        BoltType::Float(f) => serde_json::json!(f.value),
        BoltType::String(s) => serde_json::Value::String(s.value.clone()),
        BoltType::List(list) => {
            let items: Vec<serde_json::Value> = list.iter().map(bolt_value_to_json).collect();
            serde_json::Value::Array(items)
        },
        BoltType::Map(map) => bolt_map_to_json_object(map),
        BoltType::Node(node) => {
            let labels: Vec<serde_json::Value> = node
                .labels
                .iter()
                .map(|l| serde_json::Value::String(l.to_string()))
                .collect();
            let properties = bolt_map_to_json_object(&node.properties);
            serde_json::json!({
                "id": node.id.value,
                "labels": labels,
                "properties": properties,
            })
        },
        BoltType::Relation(rel) => {
            let properties = bolt_map_to_json_object(&rel.properties);
            serde_json::json!({
                "id": rel.id.value,
                "start_node_id": rel.start_node_id.value,
                "end_node_id": rel.end_node_id.value,
                "typ": rel.typ.value,
                "properties": properties,
            })
        },
        BoltType::UnboundedRelation(rel) => {
            let properties = bolt_map_to_json_object(&rel.properties);
            serde_json::json!({
                "id": rel.id.value,
                "typ": rel.typ.value,
                "properties": properties,
            })
        },
        BoltType::Path(path) => {
            let nodes: Vec<serde_json::Value> = path
                .nodes()
                .iter()
                .map(|n| {
                    let labels: Vec<serde_json::Value> = n
                        .labels
                        .iter()
                        .map(|l| serde_json::Value::String(l.to_string()))
                        .collect();
                    let properties = bolt_map_to_json_object(&n.properties);
                    serde_json::json!({
                        "id": n.id.value,
                        "labels": labels,
                        "properties": properties,
                    })
                })
                .collect();
            let rels: Vec<serde_json::Value> = path
                .rels()
                .iter()
                .map(|r| {
                    let properties = bolt_map_to_json_object(&r.properties);
                    serde_json::json!({
                        "id": r.id.value,
                        "typ": r.typ.value,
                        "properties": properties,
                    })
                })
                .collect();
            serde_json::json!({
                "nodes": nodes,
                "relationships": rels,
            })
        },
        BoltType::Point2D(p) => serde_json::json!({
            "srid": p.sr_id.value,
            "x": p.x.value,
            "y": p.y.value,
        }),
        BoltType::Point3D(p) => serde_json::json!({
            "srid": p.sr_id.value,
            "x": p.x.value,
            "y": p.y.value,
            "z": p.z.value,
        }),
        BoltType::Duration(d) => serde_json::Value::String(format!("{d:?}")),
        BoltType::Date(d) => {
            let chrono_date: chrono::NaiveDate = d
                .clone()
                .try_into()
                .unwrap_or(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
            serde_json::Value::String(chrono_date.to_string())
        },
        BoltType::Time(t) => serde_json::Value::String(format!("{t:?}")),
        BoltType::LocalTime(t) => serde_json::Value::String(format!("{t:?}")),
        BoltType::DateTime(dt) => {
            let chrono_dt: chrono::DateTime<chrono::FixedOffset> = dt.clone().try_into().unwrap_or_default();
            serde_json::Value::String(chrono_dt.to_rfc3339())
        },
        BoltType::DateTimeZoneId(dt) => serde_json::Value::String(format!("{dt:?}")),
        BoltType::LocalDateTime(dt) => {
            let chrono_dt: chrono::NaiveDateTime = dt.clone().try_into().unwrap_or_default();
            serde_json::Value::String(chrono_dt.to_string())
        },
        BoltType::Bytes(b) => serde_json::Value::String(format!("{b:?}")),
    }
}

fn bolt_map_to_json_object(map: &neo4rs::BoltMap) -> serde_json::Value {
    let obj: serde_json::Map<String, serde_json::Value> = map
        .value
        .iter()
        .map(|(k, v)| (k.value.clone(), bolt_value_to_json(v)))
        .collect();
    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo4rs::*;

    #[test]
    fn test_bolt_type_to_arrow_integer() {
        let bolt = BoltType::Integer(BoltInteger { value: 42 });
        assert_eq!(bolt_type_to_arrow(&bolt), ArrowDataType::Int64);
    }

    #[test]
    fn test_bolt_type_to_arrow_float() {
        let bolt = BoltType::Float(BoltFloat { value: 2.72 });
        assert_eq!(bolt_type_to_arrow(&bolt), ArrowDataType::Float64);
    }

    #[test]
    fn test_bolt_type_to_arrow_string() {
        let bolt = BoltType::String("hello".into());
        assert_eq!(bolt_type_to_arrow(&bolt), ArrowDataType::Utf8);
    }

    #[test]
    fn test_bolt_type_to_arrow_boolean() {
        let bolt = BoltType::Boolean(BoltBoolean { value: true });
        assert_eq!(bolt_type_to_arrow(&bolt), ArrowDataType::Boolean);
    }

    #[test]
    fn test_bolt_type_to_arrow_null() {
        let bolt = BoltType::Null(BoltNull);
        assert_eq!(bolt_type_to_arrow(&bolt), ArrowDataType::Utf8);
    }

    #[test]
    fn test_bolt_type_to_arrow_node_maps_to_utf8() {
        let bolt = BoltType::Node(BoltNode {
            id: BoltInteger { value: 1 },
            labels: BoltList::new(),
            properties: BoltMap::new(),
        });
        assert_eq!(bolt_type_to_arrow(&bolt), ArrowDataType::Utf8);
    }

    #[test]
    fn test_bolt_type_to_arrow_list_maps_to_utf8() {
        let bolt = BoltType::List(BoltList::new());
        assert_eq!(bolt_type_to_arrow(&bolt), ArrowDataType::Utf8);
    }

    #[test]
    fn test_bolt_value_to_json_null() {
        assert_eq!(bolt_value_to_json(&BoltType::Null(BoltNull)), serde_json::Value::Null);
    }

    #[test]
    fn test_bolt_value_to_json_integer() {
        let bolt = BoltType::Integer(BoltInteger { value: 42 });
        assert_eq!(bolt_value_to_json(&bolt), serde_json::json!(42));
    }

    #[test]
    fn test_bolt_value_to_json_string() {
        let bolt = BoltType::String("hello".into());
        assert_eq!(bolt_value_to_json(&bolt), serde_json::json!("hello"));
    }

    #[test]
    fn test_bolt_value_to_json_node() {
        let mut props = BoltMap::new();
        props.put("name".into(), BoltType::String("Alice".into()));

        let mut labels = BoltList::new();
        labels.push(BoltType::String("Person".into()));

        let node = BoltType::Node(BoltNode {
            id: BoltInteger { value: 1 },
            labels,
            properties: props,
        });
        let json = bolt_value_to_json(&node);
        assert_eq!(json["id"], 1);
        assert_eq!(json["labels"][0], "Person");
        assert_eq!(json["properties"]["name"], "Alice");
    }

    #[test]
    fn test_bolt_value_to_json_list() {
        let mut list = BoltList::new();
        list.push(BoltType::Integer(BoltInteger { value: 1 }));
        list.push(BoltType::Integer(BoltInteger { value: 2 }));
        let bolt = BoltType::List(list);
        assert_eq!(bolt_value_to_json(&bolt), serde_json::json!([1, 2]));
    }

    #[test]
    fn test_bolt_value_to_json_point2d() {
        let bolt = BoltType::Point2D(BoltPoint2D {
            sr_id: BoltInteger { value: 4326 },
            x: BoltFloat { value: 1.5 },
            y: BoltFloat { value: 2.5 },
        });
        let json = bolt_value_to_json(&bolt);
        assert_eq!(json["srid"], 4326);
        assert_eq!(json["x"], 1.5);
        assert_eq!(json["y"], 2.5);
    }
}
