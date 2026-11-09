use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array, PrimitiveArray,
    StringArray, TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
};
use arrow::datatypes::ArrowTimestampType;
use arrow::datatypes::DataType as ArrowDataType;
use arrow::datatypes::TimeUnit;
use arrow_cast::parse::string_to_datetime;
use chrono::Utc;

use super::errors::ConnectorError;

// Coercion rules for Boolean, integer, and float columns follow
// Elasticsearch's write-time `coerce` semantics, because sources such as ES
// return the original document (`_source`) rather than the indexed value:
// anything the source accepts is converted the way the source converts it,
// and anything it would reject is an error instead of a silent null.
//
// JSON arrays (multi-valued fields) are not unwrapped and map to null.

fn overflow_error(field_name: &str, type_name: &str, v: &serde_json::Value) -> ConnectorError {
    ConnectorError::SQLError(format!("field '{field_name}': value {v} overflows {type_name}"))
}

fn mismatch_error(field_name: &str, type_name: &str, v: &serde_json::Value) -> ConnectorError {
    ConnectorError::SQLError(format!("field '{field_name}': cannot convert {v} to {type_name}"))
}

/// Convert every value of a column with `convert`; missing values are null.
fn convert_column<T>(
    values: &[Option<&serde_json::Value>],
    convert: impl Fn(&serde_json::Value) -> Result<Option<T>, ConnectorError>,
) -> Result<Vec<Option<T>>, ConnectorError> {
    values.iter().map(|v| v.map_or(Ok(None), &convert)).collect()
}

/// Parse a numeric string, ignoring surrounding whitespace and rejecting
/// non-finite values (`"NaN"`, `"inf"`).
fn parse_numeric_str(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|f| f.is_finite())
}

/// Accepts JSON booleans and the strings `"true"`, `"false"`, and `""` (false).
fn coerce_bool(field_name: &str, v: &serde_json::Value) -> Result<Option<bool>, ConnectorError> {
    match v {
        serde_json::Value::Null | serde_json::Value::Array(_) => Ok(None),
        serde_json::Value::Bool(b) => Ok(Some(*b)),
        serde_json::Value::String(s) => match s.as_str() {
            "true" => Ok(Some(true)),
            "false" | "" => Ok(Some(false)),
            _ => Err(mismatch_error(field_name, "Boolean", v)),
        },
        _ => Err(mismatch_error(field_name, "Boolean", v)),
    }
}

/// Accepts JSON numbers and numeric strings; fractional values are truncated
/// toward zero. The result must fit in `T`.
fn coerce_int<T: TryFrom<i64>>(
    field_name: &str,
    type_name: &str,
    v: &serde_json::Value,
) -> Result<Option<T>, ConnectorError> {
    let from_f64 = |f: f64| {
        let t = f.trunc();
        // `i64::MAX as f64` rounds up to 2^63, hence the exclusive upper bound.
        if t >= i64::MIN as f64 && t < i64::MAX as f64 {
            Ok(t as i64)
        } else {
            Err(overflow_error(field_name, type_name, v))
        }
    };
    let n = match v {
        serde_json::Value::Null | serde_json::Value::Array(_) => return Ok(None),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i,
            (None, _) if n.is_u64() => return Err(overflow_error(field_name, type_name, v)),
            (None, Some(f)) => from_f64(f)?,
            (None, None) => return Err(mismatch_error(field_name, type_name, v)),
        },
        serde_json::Value::String(s) => match s.trim().parse::<i64>() {
            Ok(i) => i,
            Err(_) => match parse_numeric_str(s) {
                Some(f) => from_f64(f)?,
                None => return Err(mismatch_error(field_name, type_name, v)),
            },
        },
        _ => return Err(mismatch_error(field_name, type_name, v)),
    };
    T::try_from(n)
        .map(Some)
        .map_err(|_| overflow_error(field_name, type_name, v))
}

/// Accepts JSON numbers and numeric strings.
fn coerce_f64(field_name: &str, type_name: &str, v: &serde_json::Value) -> Result<Option<f64>, ConnectorError> {
    match v {
        serde_json::Value::Null | serde_json::Value::Array(_) => Ok(None),
        serde_json::Value::Number(n) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| mismatch_error(field_name, type_name, v)),
        serde_json::Value::String(s) => parse_numeric_str(s)
            .map(Some)
            .ok_or_else(|| mismatch_error(field_name, type_name, v)),
        _ => Err(mismatch_error(field_name, type_name, v)),
    }
}

/// Like [`coerce_f64`], but rejects values outside the `f32` range instead of
/// letting them become infinite.
fn coerce_f32(field_name: &str, v: &serde_json::Value) -> Result<Option<f32>, ConnectorError> {
    match coerce_f64(field_name, "Float32", v)? {
        Some(f) => {
            let narrowed = f as f32;
            if narrowed.is_finite() {
                Ok(Some(narrowed))
            } else {
                Err(overflow_error(field_name, "Float32", v))
            }
        },
        None => Ok(None),
    }
}

fn checked_int_array<T: TryFrom<i64>>(
    field_name: &str,
    type_name: &str,
    values: &[Option<&serde_json::Value>],
) -> Result<Vec<Option<T>>, ConnectorError> {
    convert_column(values, |v| coerce_int(field_name, type_name, v))
}

/// Convert a column of JSON values into epoch values for the given time unit.
///
/// Numbers are taken as raw epoch values in `unit`; strings are parsed as
/// ISO 8601 / RFC 3339 timestamps (naive strings are interpreted as UTC).
fn checked_timestamp_values(
    field_name: &str,
    unit: &TimeUnit,
    values: &[Option<&serde_json::Value>],
) -> Result<Vec<Option<i64>>, ConnectorError> {
    values
        .iter()
        .map(|v| match v {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Number(n)) => n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .map(Some)
                .ok_or_else(|| {
                    ConnectorError::SQLError(format!("field '{field_name}': cannot parse {n} as epoch timestamp"))
                }),
            Some(serde_json::Value::String(s)) => {
                let dt = string_to_datetime(&Utc, s)
                    .map_err(|e| ConnectorError::SQLError(format!("field '{field_name}': {e}")))?;
                let value = match unit {
                    TimeUnit::Second => Ok(dt.timestamp()),
                    TimeUnit::Millisecond => Ok(dt.timestamp_millis()),
                    TimeUnit::Microsecond => Ok(dt.timestamp_micros()),
                    TimeUnit::Nanosecond => dt.timestamp_nanos_opt().ok_or_else(|| {
                        ConnectorError::SQLError(format!("field '{field_name}': {dt} overflows nanosecond timestamps"))
                    }),
                };
                value.map(Some)
            },
            Some(other) => Err(ConnectorError::SQLError(format!(
                "field '{field_name}': cannot parse {other} as Timestamp"
            ))),
        })
        .collect()
}

/// Attach the schema's timezone to a timestamp array, leaving it timezone-naive
/// when the schema does not specify one (stamping a tz would change the logical
/// type and fail `RecordBatch::try_new` validation).
fn with_schema_timezone<P: ArrowTimestampType>(arr: PrimitiveArray<P>, tz: &Option<Arc<str>>) -> ArrayRef {
    match tz {
        Some(tz) => Arc::new(arr.with_timezone(Arc::clone(tz))),
        None => Arc::new(arr),
    }
}

/// Convert a column of JSON values into an Arrow array.
///
/// Handles Boolean, Int8/16/32/64, Float32/64, Timestamp (epoch numbers or
/// ISO 8601 / RFC 3339 strings), and falls back to Utf8.
///
/// Boolean and numeric columns also accept string forms (`"true"`, `"42"`,
/// `"1.5"`), and integer columns truncate fractional values toward zero.
/// Values that cannot be converted, or that overflow the target type, are
/// errors rather than nulls.
pub fn json_values_to_array(
    field_name: &str,
    data_type: &ArrowDataType,
    values: &[Option<&serde_json::Value>],
) -> Result<ArrayRef, ConnectorError> {
    match data_type {
        ArrowDataType::Boolean => {
            let arr: BooleanArray = convert_column(values, |v| coerce_bool(field_name, v))?
                .into_iter()
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int8 => {
            let arr: Int8Array = checked_int_array(field_name, "Int8", values)?.into_iter().collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int16 => {
            let arr: Int16Array = checked_int_array(field_name, "Int16", values)?.into_iter().collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int32 => {
            let arr: Int32Array = checked_int_array(field_name, "Int32", values)?.into_iter().collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Int64 => {
            let arr: Int64Array = checked_int_array(field_name, "Int64", values)?.into_iter().collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Float32 => {
            let arr: Float32Array = convert_column(values, |v| coerce_f32(field_name, v))?
                .into_iter()
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Float64 => {
            let arr: Float64Array = convert_column(values, |v| coerce_f64(field_name, "Float64", v))?
                .into_iter()
                .collect();
            Ok(Arc::new(arr))
        },
        ArrowDataType::Timestamp(unit, tz) => {
            let epoch_values = checked_timestamp_values(field_name, unit, values)?;
            Ok(match unit {
                TimeUnit::Second => {
                    let arr: TimestampSecondArray = epoch_values.into_iter().collect();
                    with_schema_timezone(arr, tz)
                },
                TimeUnit::Millisecond => {
                    let arr: TimestampMillisecondArray = epoch_values.into_iter().collect();
                    with_schema_timezone(arr, tz)
                },
                TimeUnit::Microsecond => {
                    let arr: TimestampMicrosecondArray = epoch_values.into_iter().collect();
                    with_schema_timezone(arr, tz)
                },
                TimeUnit::Nanosecond => {
                    let arr: TimestampNanosecondArray = epoch_values.into_iter().collect();
                    with_schema_timezone(arr, tz)
                },
            })
        },
        _ => {
            let arr: StringArray = values
                .iter()
                .map(|v| {
                    v.and_then(|v| match v {
                        serde_json::Value::Null => None,
                        serde_json::Value::String(s) => Some(s.clone()),
                        other => Some(other.to_string()),
                    })
                })
                .collect();
            Ok(Arc::new(arr))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;

    #[test]
    fn test_boolean() {
        let t = serde_json::json!(true);
        let f = serde_json::json!(false);
        let vals = vec![Some(&t), None, Some(&f)];
        let arr = json_values_to_array("flag", &ArrowDataType::Boolean, &vals).unwrap();
        let bool_arr = arr.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert!(bool_arr.value(0));
        assert!(bool_arr.is_null(1));
        assert!(!bool_arr.value(2));
    }

    #[test]
    fn test_int8_boundary() {
        let v_min = serde_json::json!(-128);
        let v_max = serde_json::json!(127);
        let vals = vec![Some(&v_min), None, Some(&v_max)];
        let arr = json_values_to_array("score", &ArrowDataType::Int8, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int8Array>().unwrap();
        assert_eq!(int_arr.value(0), -128);
        assert!(int_arr.is_null(1));
        assert_eq!(int_arr.value(2), 127);
    }

    #[test]
    fn test_int16_boundary() {
        let v_min = serde_json::json!(-32768);
        let v_max = serde_json::json!(32767);
        let vals = vec![Some(&v_min), Some(&v_max)];
        let arr = json_values_to_array("val", &ArrowDataType::Int16, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int16Array>().unwrap();
        assert_eq!(int_arr.value(0), -32768);
        assert_eq!(int_arr.value(1), 32767);
    }

    #[test]
    fn test_int32_boundary() {
        let v_min = serde_json::json!(-2_147_483_648_i64);
        let v_max = serde_json::json!(2_147_483_647_i64);
        let vals = vec![Some(&v_min), Some(&v_max)];
        let arr = json_values_to_array("id", &ArrowDataType::Int32, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(int_arr.value(0), -2_147_483_648);
        assert_eq!(int_arr.value(1), 2_147_483_647);
    }

    #[test]
    fn test_int64() {
        let v = serde_json::json!(42);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array("count", &ArrowDataType::Int64, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(int_arr.value(0), 42);
        assert!(int_arr.is_null(1));
    }

    #[test]
    fn test_float32() {
        let v = serde_json::json!(1.23);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array("score", &ArrowDataType::Float32, &vals).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float32Array>().unwrap();
        assert!((f_arr.value(0) - 1.23f32).abs() < f32::EPSILON);
        assert!(f_arr.is_null(1));
    }

    #[test]
    fn test_float64() {
        let v = serde_json::json!(1.23);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array("price", &ArrowDataType::Float64, &vals).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((f_arr.value(0) - 1.23).abs() < f64::EPSILON);
        assert!(f_arr.is_null(1));
    }

    #[test]
    fn test_utf8_fallback() {
        let v_str = serde_json::json!("hello");
        let v_obj = serde_json::json!({"nested": true});
        let vals = vec![Some(&v_str), Some(&v_obj), None];
        let arr = json_values_to_array("data", &ArrowDataType::Utf8, &vals).unwrap();
        let str_arr = arr.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(str_arr.value(0), "hello");
        assert_eq!(str_arr.value(1), r#"{"nested":true}"#);
        assert!(str_arr.is_null(2));
    }

    #[test]
    fn test_utf8_json_null_is_null() {
        let v_null = serde_json::Value::Null;
        let v_str = serde_json::json!("ok");
        let vals = vec![Some(&v_null), Some(&v_str), None];
        let arr = json_values_to_array("title", &ArrowDataType::Utf8, &vals).unwrap();
        let str_arr = arr.as_any().downcast_ref::<StringArray>().unwrap();
        assert!(str_arr.is_null(0), "JSON null should map to Arrow null, not \"null\"");
        assert_eq!(str_arr.value(1), "ok");
        assert!(str_arr.is_null(2));
    }

    #[test]
    fn test_timestamp_epoch_millis() {
        let v = serde_json::json!(1700000000000_i64);
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
        assert!(ts_arr.is_null(1));
    }

    #[test]
    fn test_timestamp_iso_z() {
        let v = serde_json::json!("2023-11-14T22:13:20.000Z");
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
    }

    #[test]
    fn test_timestamp_naive_iso_is_utc() {
        let v = serde_json::json!("2023-11-14T22:13:20");
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
    }

    #[test]
    fn test_timestamp_offset() {
        let v = serde_json::json!("2023-11-15T04:13:20+06:00");
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
    }

    #[test]
    fn test_boolean_coerces_strings() {
        let vals_json = [
            serde_json::json!("true"),
            serde_json::json!("false"),
            serde_json::json!(""),
            serde_json::Value::Null,
        ];
        let vals: Vec<_> = vals_json.iter().map(Some).collect();
        let arr = json_values_to_array("flag", &ArrowDataType::Boolean, &vals).unwrap();
        let bool_arr = arr.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert!(bool_arr.value(0));
        assert!(!bool_arr.value(1));
        assert!(!bool_arr.value(2), "empty string is false");
        assert!(bool_arr.is_null(3));
    }

    #[test]
    fn test_int_truncates_fractions_toward_zero() {
        let vals_json = [
            serde_json::json!(3.7),
            serde_json::json!(-3.7),
            serde_json::json!(300.0),
            serde_json::json!("3.7"),
            serde_json::json!("-3.7"),
            serde_json::json!("1e3"),
        ];
        let vals: Vec<_> = vals_json.iter().map(Some).collect();
        let arr = json_values_to_array("count", &ArrowDataType::Int16, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int16Array>().unwrap();
        assert_eq!(int_arr.values(), &[3, -3, 300, 3, -3, 1000]);
    }

    #[test]
    fn test_int_coerces_numeric_strings() {
        let vals_json = [
            serde_json::json!("300"),
            serde_json::json!(" 42 "),
            serde_json::json!("-7"),
        ];
        let vals: Vec<_> = vals_json.iter().map(Some).collect();
        let arr = json_values_to_array("count", &ArrowDataType::Int32, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int32Array>().unwrap();
        assert_eq!(int_arr.values(), &[300, 42, -7]);
    }

    #[test]
    fn test_int64_boundaries_from_numbers_and_strings() {
        let vals_json = [
            serde_json::json!(i64::MIN),
            serde_json::json!(i64::MAX),
            serde_json::json!("-9223372036854775808"),
            serde_json::json!("9223372036854775807"),
        ];
        let vals: Vec<_> = vals_json.iter().map(Some).collect();
        let arr = json_values_to_array("id", &ArrowDataType::Int64, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(int_arr.values(), &[i64::MIN, i64::MAX, i64::MIN, i64::MAX]);
    }

    #[test]
    fn test_int_truncation_stays_in_range() {
        let v = serde_json::json!(127.9);
        let vals = vec![Some(&v)];
        let arr = json_values_to_array("score", &ArrowDataType::Int8, &vals).unwrap();
        let int_arr = arr.as_any().downcast_ref::<Int8Array>().unwrap();
        assert_eq!(int_arr.value(0), 127);
    }

    #[test]
    fn test_float_coerces_numeric_strings() {
        let vals_json = [serde_json::json!("1.5"), serde_json::json!(" -2 ")];
        let vals: Vec<_> = vals_json.iter().map(Some).collect();
        let arr = json_values_to_array("price", &ArrowDataType::Float64, &vals).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float64Array>().unwrap();
        assert_eq!(f_arr.values(), &[1.5, -2.0]);

        let arr = json_values_to_array("price", &ArrowDataType::Float32, &vals).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float32Array>().unwrap();
        assert_eq!(f_arr.values(), &[1.5f32, -2.0f32]);
    }

    #[test]
    fn test_float32_max_is_accepted() {
        let v = serde_json::json!(f64::from(f32::MAX));
        let vals = vec![Some(&v)];
        let arr = json_values_to_array("score", &ArrowDataType::Float32, &vals).unwrap();
        let f_arr = arr.as_any().downcast_ref::<Float32Array>().unwrap();
        assert_eq!(f_arr.value(0), f32::MAX);
    }

    #[test]
    fn test_arrays_map_to_null() {
        let v = serde_json::json!([1, 2]);
        let vals = vec![Some(&v)];
        for dt in [
            ArrowDataType::Boolean,
            ArrowDataType::Int8,
            ArrowDataType::Int64,
            ArrowDataType::Float32,
            ArrowDataType::Float64,
        ] {
            let arr = json_values_to_array("tags", &dt, &vals).unwrap();
            assert!(arr.is_null(0), "{dt} should map an array to null");
        }
    }

    #[test]
    fn test_timestamp_epoch_secs_unit() {
        let v = serde_json::json!(1700000000_i64);
        let vals = vec![Some(&v)];
        let arr = json_values_to_array("created_at", &ArrowDataType::Timestamp(TimeUnit::Second, None), &vals).unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampSecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000);
        assert_eq!(ts_arr.data_type(), &ArrowDataType::Timestamp(TimeUnit::Second, None));
    }

    #[test]
    fn test_timestamp_json_null() {
        let v = serde_json::Value::Null;
        let vals = vec![Some(&v), None];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert!(ts_arr.is_null(0));
        assert!(ts_arr.is_null(1));
    }

    #[test]
    fn test_timestamp_timezone_label_propagates() {
        let v = serde_json::json!("2023-11-14T22:13:20Z");
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("America/New_York".into())),
            &vals,
        )
        .unwrap();
        // The instant is stored the same regardless of label; the schema's tz
        // must appear on the array's data type so RecordBatch validation passes.
        let ts_arr = arr.as_any().downcast_ref::<TimestampMillisecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000);
        assert_eq!(
            ts_arr.data_type(),
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("America/New_York".into()))
        );
    }

    #[test]
    fn test_timestamp_epoch_micros_and_nanos_units() {
        let v = serde_json::json!(1700000000000000_i64);
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000000);

        let v = serde_json::json!(1700000000000000000_i64);
        let vals = vec![Some(&v)];
        let arr = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
            &vals,
        )
        .unwrap();
        let ts_arr = arr.as_any().downcast_ref::<TimestampNanosecondArray>().unwrap();
        assert_eq!(ts_arr.value(0), 1700000000000000000);
    }

    #[test]
    fn test_timestamp_nanos_overflow_from_string() {
        // "9999-12-31T23:59:59Z" far exceeds i64 nanoseconds.
        let v = serde_json::json!("9999-12-31T23:59:59Z");
        let vals = vec![Some(&v)];
        let err = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
            &vals,
        )
        .unwrap_err();
        assert!(
            matches!(err, ConnectorError::SQLError(_)),
            "should be SQLError: {}",
            err
        );
    }

    #[test]
    fn test_timestamp_invalid_string() {
        let v = serde_json::json!("not-a-date");
        let vals = vec![Some(&v)];
        let err = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, ConnectorError::SQLError(_)), "should be SQLError: {msg}");
        assert!(msg.contains("created_at"), "should contain field name: {msg}");
    }

    #[test]
    fn test_timestamp_wrong_type() {
        let v = serde_json::json!(true);
        let vals = vec![Some(&v)];
        let err = json_values_to_array(
            "created_at",
            &ArrowDataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &vals,
        )
        .unwrap_err();
        assert!(
            matches!(err, ConnectorError::SQLError(_)),
            "should be SQLError: {}",
            err
        );
    }

    macro_rules! conversion_error_test {
        ($name:ident, $dt:expr, $val:expr, $type_str:expr) => {
            #[test]
            fn $name() {
                let v = serde_json::json!($val);
                let vals = vec![Some(&v)];
                let err = json_values_to_array("my_field", &$dt, &vals).unwrap_err();
                let msg = err.to_string();
                assert!(
                    matches!(err, ConnectorError::SQLError(_)),
                    "should be SQLError: {msg}"
                );
                assert!(msg.contains("my_field"), "should contain field name: {msg}");
                assert!(msg.contains($type_str), "should contain type: {msg}");
            }
        };
    }

    conversion_error_test!(test_int8_overflow_pos, ArrowDataType::Int8, 300, "Int8");
    conversion_error_test!(test_int8_overflow_neg, ArrowDataType::Int8, -129, "Int8");
    conversion_error_test!(test_int16_overflow_pos, ArrowDataType::Int16, 32768, "Int16");
    conversion_error_test!(test_int16_overflow_neg, ArrowDataType::Int16, -32769, "Int16");
    conversion_error_test!(
        test_int32_overflow_pos,
        ArrowDataType::Int32,
        2_147_483_648_i64,
        "Int32"
    );
    conversion_error_test!(
        test_int32_overflow_neg,
        ArrowDataType::Int32,
        -2_147_483_649_i64,
        "Int32"
    );
    conversion_error_test!(test_int8_overflow_from_string, ArrowDataType::Int8, "300", "Int8");
    conversion_error_test!(test_int8_overflow_after_truncation, ArrowDataType::Int8, 128.5, "Int8");
    conversion_error_test!(test_int64_overflow_u64, ArrowDataType::Int64, u64::MAX, "Int64");
    conversion_error_test!(test_int64_overflow_float, ArrowDataType::Int64, 1e20, "Int64");
    conversion_error_test!(
        test_int64_overflow_long_string,
        ArrowDataType::Int64,
        "99999999999999999999",
        "Int64"
    );
    conversion_error_test!(test_float32_overflow_pos, ArrowDataType::Float32, 1e300, "Float32");
    conversion_error_test!(test_float32_overflow_neg, ArrowDataType::Float32, -1e300, "Float32");
    conversion_error_test!(
        test_float32_overflow_from_string,
        ArrowDataType::Float32,
        "1e300",
        "Float32"
    );

    // Values the source would reject at write time are errors, not nulls.
    conversion_error_test!(test_boolean_rejects_number, ArrowDataType::Boolean, 1, "Boolean");
    conversion_error_test!(
        test_boolean_rejects_other_string,
        ArrowDataType::Boolean,
        "yes",
        "Boolean"
    );
    conversion_error_test!(
        test_int_rejects_non_numeric_string,
        ArrowDataType::Int32,
        "abc",
        "Int32"
    );
    conversion_error_test!(test_int_rejects_empty_string, ArrowDataType::Int32, "", "Int32");
    conversion_error_test!(test_int_rejects_nan_string, ArrowDataType::Int64, "NaN", "Int64");
    conversion_error_test!(test_int_rejects_bool, ArrowDataType::Int64, true, "Int64");
    conversion_error_test!(
        test_int_rejects_object,
        ArrowDataType::Int64,
        serde_json::json!({"a": 1}),
        "Int64"
    );
    conversion_error_test!(
        test_float_rejects_non_numeric_string,
        ArrowDataType::Float64,
        "abc",
        "Float64"
    );
    conversion_error_test!(test_float_rejects_inf_string, ArrowDataType::Float64, "inf", "Float64");
    conversion_error_test!(test_float_rejects_bool, ArrowDataType::Float32, false, "Float32");
}
