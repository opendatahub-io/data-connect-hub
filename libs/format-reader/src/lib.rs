mod binary;
mod csv;
mod decode;
mod decompress;
mod detect;
mod jsonl;
mod parquet;

pub use binary::{binary_schema, read_binary_batches};
pub use csv::{read_csv_batches, read_csv_schema};
pub use decode::ByteStream;
pub use decompress::decompress_stream;
pub use detect::{Compression, FileFormat};
pub use jsonl::{read_jsonl_batches, read_jsonl_schema};
pub use parquet::{read_parquet_batches, read_parquet_schema};
