use crate::decode::ByteStream;
use crate::detect::Compression;
use tokio_util::io::{ReaderStream, StreamReader};

pub fn decompress_stream(stream: ByteStream, compression: Compression) -> ByteStream {
    match compression {
        Compression::None => stream,
        Compression::Gzip => {
            let reader = StreamReader::new(stream);
            let decoder = async_compression::tokio::bufread::GzipDecoder::new(tokio::io::BufReader::new(reader));
            Box::pin(ReaderStream::new(decoder))
        },
        Compression::Zstd => {
            let reader = StreamReader::new(stream);
            let decoder = async_compression::tokio::bufread::ZstdDecoder::new(tokio::io::BufReader::new(reader));
            Box::pin(ReaderStream::new(decoder))
        },
    }
}
