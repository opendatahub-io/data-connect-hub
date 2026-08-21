pub mod parquet;

use commons::api::errors::ConnectorError;
use format_reader::ByteStream;
use futures::TryStreamExt;
use opendal::Reader;

pub use format_reader::FileFormat;

pub async fn opendal_to_byte_stream(reader: Reader) -> Result<ByteStream, ConnectorError> {
    let stream = reader
        .into_stream(..)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to open stream: {e}")))?;
    Ok(Box::pin(
        stream.map_ok(|buf| buf.to_bytes()).map_err(std::io::Error::other),
    ))
}
