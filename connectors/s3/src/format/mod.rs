mod csv;
mod jsonl;
mod parquet;

use commons::api::connector::QueryOutput;
use commons::api::errors::ConnectorError;
use futures::TryStreamExt;
use opendal::Reader;

pub use csv::{read_csv_batches, read_csv_schema};
pub use jsonl::{read_jsonl_batches, read_jsonl_schema};
pub use parquet::{read_parquet_batches, read_parquet_schema};

const MAX_SCHEMA_SAMPLE: usize = 1_048_576;

enum Decoder {
    Csv(Box<arrow_csv::reader::Decoder>),
    Json(Box<arrow_json::reader::Decoder>),
}

impl Decoder {
    fn decode(&mut self, buf: &[u8]) -> Result<usize, arrow::error::ArrowError> {
        match self {
            Decoder::Csv(d) => d.decode(buf),
            Decoder::Json(d) => d.decode(buf),
        }
    }

    fn flush(&mut self) -> Result<Option<arrow::record_batch::RecordBatch>, arrow::error::ArrowError> {
        match self {
            Decoder::Csv(d) => d.flush(),
            Decoder::Json(d) => d.flush(),
        }
    }
}

async fn decode_stream(reader: Reader, mut decoder: Decoder, format: &str) -> QueryOutput {
    let mut buf_stream = reader
        .into_stream(..)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to open {format} stream: {e}")))?;

    let format = format.to_owned();
    let stream = async_stream::try_stream! {
        while let Some(buf) = buf_stream
            .try_next()
            .await
            .map_err(|e| ConnectorError::IOError(format!("{format} stream read error: {e}")))?
        {
            let chunk = buf.to_bytes();
            let mut offset = 0;
            while offset < chunk.len() {
                let consumed = decoder
                    .decode(&chunk[offset..])
                    .map_err(|e| ConnectorError::IOError(format!("{format} decode error: {e}")))?;

                if consumed == 0 {
                    if let Some(batch) = decoder
                        .flush()
                        .map_err(|e| ConnectorError::IOError(format!("{format} flush error: {e}")))? {
                        yield batch;
                    }
                } else {
                    offset += consumed;
                }
            }
        }

        if let Some(batch) = decoder
            .flush()
            .map_err(|e| ConnectorError::IOError(format!("{format} flush error: {e}")))? {
            yield batch;
        }
    };

    Ok(Box::pin(stream))
}

async fn read_sample(reader: Reader) -> Result<Vec<u8>, ConnectorError> {
    let mut buf_stream = reader
        .into_stream(..)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to open stream: {e}")))?;

    let mut buf = Vec::new();
    while let Some(chunk) = buf_stream
        .try_next()
        .await
        .map_err(|e| ConnectorError::IOError(format!("Stream read error: {e}")))?
    {
        buf.extend_from_slice(&chunk.to_bytes());
        if buf.len() >= MAX_SCHEMA_SAMPLE {
            break;
        }
    }

    if let Some(pos) = buf.iter().rposition(|&b| b == b'\n') {
        buf.truncate(pos + 1);
    }

    Ok(buf)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileFormat {
    Parquet,
    Csv,
    JsonLines,
}

impl FileFormat {
    pub fn detect(path: &str, properties: Option<&str>) -> Result<Self, ConnectorError> {
        if let Some(fmt) = properties {
            return match fmt.to_lowercase().as_str() {
                "parquet" => Ok(FileFormat::Parquet),
                "csv" => Ok(FileFormat::Csv),
                "jsonl" | "ndjson" | "jsonlines" => Ok(FileFormat::JsonLines),
                other => Err(ConnectorError::InvalidRequest(format!("Unsupported format: {other}"))),
            };
        }

        let lower = path.to_lowercase();
        if lower.ends_with(".parquet") {
            Ok(FileFormat::Parquet)
        } else if lower.ends_with(".csv") {
            Ok(FileFormat::Csv)
        } else if lower.ends_with(".jsonl") || lower.ends_with(".ndjson") || lower.ends_with(".jsonlines") {
            Ok(FileFormat::JsonLines)
        } else {
            Err(ConnectorError::InvalidRequest(format!(
                "Cannot detect format for path: {path}. Set 'format' in connection properties."
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_format_from_properties() {
        assert_eq!(
            FileFormat::detect("anything", Some("parquet")).unwrap(),
            FileFormat::Parquet
        );
        assert_eq!(FileFormat::detect("anything", Some("csv")).unwrap(), FileFormat::Csv);
        assert_eq!(
            FileFormat::detect("anything", Some("jsonl")).unwrap(),
            FileFormat::JsonLines
        );
        assert_eq!(
            FileFormat::detect("anything", Some("ndjson")).unwrap(),
            FileFormat::JsonLines
        );
        assert_eq!(
            FileFormat::detect("anything", Some("jsonlines")).unwrap(),
            FileFormat::JsonLines
        );
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
            FileFormat::detect("data/train.jsonl", None).unwrap(),
            FileFormat::JsonLines
        );
        assert_eq!(
            FileFormat::detect("data/train.ndjson", None).unwrap(),
            FileFormat::JsonLines
        );
        assert_eq!(
            FileFormat::detect("data/train.jsonlines", None).unwrap(),
            FileFormat::JsonLines
        );
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
}
