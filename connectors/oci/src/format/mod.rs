pub mod parquet;

pub use format_reader::{
    ByteStream, Compression, FileFormat, binary_schema, decompress_stream, read_binary_batches, read_csv_batches,
    read_csv_schema, read_jsonl_batches, read_jsonl_schema, read_parquet_batches, read_parquet_schema,
};
