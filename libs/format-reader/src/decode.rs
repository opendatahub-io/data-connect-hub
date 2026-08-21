use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use commons::api::errors::ConnectorError;
use commons::api::tabular::QueryOutput;
use futures::TryStreamExt;
use futures::stream::BoxStream;

pub type ByteStream = BoxStream<'static, Result<Bytes, std::io::Error>>;

pub(crate) const MAX_SCHEMA_SAMPLE: usize = 1_048_576;

pub(crate) enum Decoder {
    Csv(Box<arrow_csv::reader::Decoder>),
    Json(Box<arrow_json::reader::Decoder>),
}

impl Decoder {
    pub(crate) fn decode(&mut self, buf: &[u8]) -> Result<usize, arrow::error::ArrowError> {
        match self {
            Decoder::Csv(d) => d.decode(buf),
            Decoder::Json(d) => d.decode(buf),
        }
    }

    pub(crate) fn flush(&mut self) -> Result<Option<RecordBatch>, arrow::error::ArrowError> {
        match self {
            Decoder::Csv(d) => d.flush(),
            Decoder::Json(d) => d.flush(),
        }
    }
}

pub(crate) async fn decode_stream(mut stream: ByteStream, mut decoder: Decoder, format: &str) -> QueryOutput {
    let format = format.to_owned();
    let output = async_stream::try_stream! {
        while let Some(buf) = stream
            .try_next()
            .await
            .map_err(|e| ConnectorError::IOError(format!("{format} stream read error: {e}")))?
        {
            let mut offset = 0;
            while offset < buf.len() {
                let consumed = decoder
                    .decode(&buf[offset..])
                    .map_err(|e| ConnectorError::IOError(format!("{format} decode error: {e}")))?;

                if consumed == 0 {
                    if let Some(batch) = decoder
                        .flush()
                        .map_err(|e| ConnectorError::IOError(format!("{format} flush error: {e}")))? {
                        yield batch;
                    } else {
                        break;
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

    Ok(Box::pin(output))
}

pub(crate) async fn read_sample(mut stream: ByteStream) -> Result<Vec<u8>, ConnectorError> {
    let mut buf = Vec::new();
    while let Some(chunk) = stream
        .try_next()
        .await
        .map_err(|e| ConnectorError::IOError(format!("Stream read error: {e}")))?
    {
        buf.extend_from_slice(&chunk);
        if buf.len() >= MAX_SCHEMA_SAMPLE {
            break;
        }
    }

    if let Some(pos) = buf.iter().rposition(|&b| b == b'\n') {
        buf.truncate(pos + 1);
    }

    Ok(buf)
}
