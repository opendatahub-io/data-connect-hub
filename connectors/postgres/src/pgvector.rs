//! pgvector extension type support: Arrow type mapping and binary decoding
//! for `vector`, `halfvec` and `sparsevec` columns.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Builder, ListBuilder, StringArray};
use arrow::datatypes::{ArrowPrimitiveType, DataType, Field, Float16Type};
use commons::api::errors::ConnectorError;
use sqlx::postgres::{PgRow, PgValueFormat};
use sqlx::{Row, TypeInfo, ValueRef};

use crate::connector::map_sqlx_error;

// pgvector type names are lowercase, unlike the uppercase built-in names:
// sqlx reports built-in types from its static table (`INT4`, `FLOAT8`)
// but names extension types by their `regtype` text, which is lowercase.
pub(crate) const VECTOR: &str = "vector";
pub(crate) const HALFVEC: &str = "halfvec";
pub(crate) const SPARSEVEC: &str = "sparsevec";

/// Arrow type for pgvector `vector` and `halfvec` columns. `halfvec` is
/// widened to `Float32` (lossless) for broad consumer compatibility.
pub(crate) fn list_type() -> DataType {
    DataType::List(Arc::new(Field::new_list_field(DataType::Float32, true)))
}

/// Build an Arrow array for a `vector` column.
pub(crate) fn build_vector_array(rows: &[PgRow], col_idx: usize) -> Result<ArrayRef, ConnectorError> {
    build_list_array(rows, col_idx, decode_vector)
}

/// Build an Arrow array for a `halfvec` column.
pub(crate) fn build_halfvec_array(rows: &[PgRow], col_idx: usize) -> Result<ArrayRef, ConnectorError> {
    build_list_array(rows, col_idx, decode_halfvec)
}

/// Build an Arrow array for a `sparsevec` column, using its text representation.
pub(crate) fn build_sparsevec_array(rows: &[PgRow], col_idx: usize) -> Result<ArrayRef, ConnectorError> {
    let strs: Vec<Option<String>> = raw_binary_values(rows, col_idx)?
        .into_iter()
        .map(|v| v.map(decode_sparsevec).transpose())
        .collect::<Result<_, _>>()
        .map_err(ConnectorError::SQLError)?;
    Ok(Arc::new(StringArray::from(strs)))
}

fn read_be_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn read_be_i32(bytes: &[u8], offset: usize) -> i32 {
    read_be_u32(bytes, offset) as i32
}

fn check_len(type_name: &str, bytes: &[u8], expected: usize) -> Result<(), String> {
    if bytes.len() != expected {
        return Err(format!(
            "invalid {type_name} binary value: expected {expected} bytes, got {}",
            bytes.len()
        ));
    }
    Ok(())
}

/// Decode pgvector `vector` binary format: `int16 dim, int16 unused, float4[dim]`.
fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() < 4 {
        return Err(format!("invalid vector binary value: {} bytes", bytes.len()));
    }
    let dim = read_be_u16(bytes, 0) as usize;
    check_len(VECTOR, bytes, 4 + dim * 4)?;
    Ok((0..dim)
        .map(|i| f32::from_bits(read_be_u32(bytes, 4 + i * 4)))
        .collect())
}

/// Decode pgvector `halfvec` binary format: `int16 dim, int16 unused, half[dim]`.
fn decode_halfvec(bytes: &[u8]) -> Result<Vec<f32>, String> {
    type F16 = <Float16Type as ArrowPrimitiveType>::Native;
    if bytes.len() < 4 {
        return Err(format!("invalid halfvec binary value: {} bytes", bytes.len()));
    }
    let dim = read_be_u16(bytes, 0) as usize;
    check_len(HALFVEC, bytes, 4 + dim * 2)?;
    Ok((0..dim)
        .map(|i| F16::from_bits(read_be_u16(bytes, 4 + i * 2)).to_f32())
        .collect())
}

/// Decode pgvector `sparsevec` binary format
/// (`int32 dim, int32 nnz, int32 unused, int32[nnz] indices, float4[nnz] values`)
/// into its text representation, e.g. `{1:1.5,3:2.5}/5` (1-based indices).
fn decode_sparsevec(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() < 12 {
        return Err(format!("invalid sparsevec binary value: {} bytes", bytes.len()));
    }
    let dim = read_be_i32(bytes, 0);
    let nnz = usize::try_from(read_be_i32(bytes, 4)).map_err(|_| "invalid sparsevec nnz".to_string())?;
    check_len(SPARSEVEC, bytes, 12 + nnz * 8)?;
    let values_offset = 12 + nnz * 4;
    let elements: Vec<String> = (0..nnz)
        .map(|i| {
            let index = read_be_i32(bytes, 12 + i * 4) as i64 + 1;
            let value = f32::from_bits(read_be_u32(bytes, values_offset + i * 4));
            format!("{index}:{value}")
        })
        .collect();
    Ok(format!("{{{}}}/{dim}", elements.join(",")))
}

/// Read the raw binary bytes of each row's column value, `None` for NULL.
///
/// Relies on rows being fetched in binary format: `sqlx::query()` always
/// carries (possibly empty) arguments, so sqlx executes it as a prepared
/// statement with binary result formats. The simple query protocol
/// (`sqlx::raw_sql`) returns text instead, which this rejects explicitly.
fn raw_binary_values(rows: &[PgRow], col_idx: usize) -> Result<Vec<Option<&[u8]>>, ConnectorError> {
    rows.iter()
        .map(|r| {
            let value = r.try_get_raw(col_idx).map_err(map_sqlx_error)?;
            if value.is_null() {
                return Ok(None);
            }
            if value.format() != PgValueFormat::Binary {
                return Err(ConnectorError::SQLError(format!(
                    "unexpected text format for column {col_idx} of type {}",
                    value.type_info().name()
                )));
            }
            value
                .as_bytes()
                .map(Some)
                .map_err(|e| ConnectorError::SQLError(e.to_string()))
        })
        .collect()
}

/// Decoder from a pgvector binary value to its `f32` elements.
type VectorDecoder = fn(&[u8]) -> Result<Vec<f32>, String>;

fn build_list_array(rows: &[PgRow], col_idx: usize, decode: VectorDecoder) -> Result<ArrayRef, ConnectorError> {
    let mut builder = ListBuilder::new(Float32Builder::new());
    for value in raw_binary_values(rows, col_idx)? {
        match value {
            Some(bytes) => {
                let vals = decode(bytes).map_err(ConnectorError::SQLError)?;
                builder.values().append_slice(&vals);
                builder.append(true);
            },
            None => builder.append(false),
        }
    }
    Ok(Arc::new(builder.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector_bytes(vals: &[f32]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&(vals.len() as u16).to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        for v in vals {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b
    }

    #[test]
    fn test_decode_vector() {
        let vals = [0.1f32, -2.0, 3.5];
        assert_eq!(decode_vector(&vector_bytes(&vals)).unwrap(), vals);
    }

    #[test]
    fn test_decode_vector_empty() {
        assert_eq!(decode_vector(&vector_bytes(&[])).unwrap(), Vec::<f32>::new());
    }

    #[test]
    fn test_decode_vector_invalid_length() {
        let mut b = vector_bytes(&[1.0, 2.0]);
        b.pop();
        assert!(decode_vector(&b).is_err());
        assert!(decode_vector(&[0, 1]).is_err());
    }

    #[test]
    fn test_decode_halfvec() {
        // 1.0 = 0x3C00, -2.0 = 0xC000, 0.5 = 0x3800 in IEEE 754 half precision.
        let b = [0, 3, 0, 0, 0x3C, 0x00, 0xC0, 0x00, 0x38, 0x00];
        assert_eq!(decode_halfvec(&b).unwrap(), vec![1.0, -2.0, 0.5]);
        assert!(decode_halfvec(&b[..9]).is_err());
    }

    #[test]
    fn test_decode_sparsevec() {
        let mut b = Vec::new();
        for v in [5i32, 2, 0, 0, 2] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        for v in [1.5f32, 2.5] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(decode_sparsevec(&b).unwrap(), "{1:1.5,3:2.5}/5");
        assert!(decode_sparsevec(&b[..b.len() - 1]).is_err());
    }

    #[test]
    fn test_decode_sparsevec_empty() {
        let mut b = Vec::new();
        for v in [3i32, 0, 0] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(decode_sparsevec(&b).unwrap(), "{}/3");
    }
}
