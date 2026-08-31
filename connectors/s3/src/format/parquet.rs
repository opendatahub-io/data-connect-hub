use std::ops::Range;
use std::sync::Arc;

use arrow::datatypes::Schema;
use bytes::Bytes;
use commons::api::connector::QueryOutput;
use commons::api::errors::ConnectorError;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use opendal::{BytesRange, Reader};
use parquet::arrow::async_reader::{AsyncFileReader, MetadataSuffixFetch, ParquetRecordBatchStreamBuilder};
use parquet::file::metadata::ParquetMetaData;

struct OpendalAsyncReader {
    reader: Reader,
}

impl OpendalAsyncReader {
    fn new(reader: Reader) -> Self {
        Self { reader }
    }
}

impl AsyncFileReader for OpendalAsyncReader {
    fn get_bytes(&mut self, range: Range<u64>) -> BoxFuture<'_, parquet::errors::Result<Bytes>> {
        async move {
            let buf = self
                .reader
                .read(range)
                .await
                .map_err(|e| parquet::errors::ParquetError::External(Box::new(e)))?;
            Ok(buf.to_bytes())
        }
        .boxed()
    }

    fn get_byte_ranges(&mut self, ranges: Vec<Range<u64>>) -> BoxFuture<'_, parquet::errors::Result<Vec<Bytes>>> {
        async move {
            let bufs = self
                .reader
                .fetch(ranges)
                .await
                .map_err(|e| parquet::errors::ParquetError::External(Box::new(e)))?;
            Ok(bufs.into_iter().map(|b| b.to_bytes()).collect())
        }
        .boxed()
    }

    fn get_metadata<'a>(
        &'a mut self,
        options: Option<&'a parquet::arrow::arrow_reader::ArrowReaderOptions>,
    ) -> BoxFuture<'a, parquet::errors::Result<Arc<ParquetMetaData>>> {
        async move {
            let metadata_reader =
                parquet::file::metadata::ParquetMetaDataReader::new().with_arrow_reader_options(options);
            let parquet_metadata = metadata_reader.load_via_suffix_and_finish(self).await?;
            Ok(Arc::new(parquet_metadata))
        }
        .boxed()
    }
}

impl MetadataSuffixFetch for &mut OpendalAsyncReader {
    fn fetch_suffix(&mut self, suffix: usize) -> BoxFuture<'_, parquet::errors::Result<Bytes>> {
        async move {
            let buf = self
                .reader
                .read(BytesRange::suffix(suffix as u64))
                .await
                .map_err(|e| parquet::errors::ParquetError::External(Box::new(e)))?;
            Ok(buf.to_bytes())
        }
        .boxed()
    }
}

pub async fn read_parquet_schema(reader: Reader) -> Result<Schema, ConnectorError> {
    let async_reader = OpendalAsyncReader::new(reader);
    let builder = ParquetRecordBatchStreamBuilder::new(async_reader)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to read Parquet metadata: {e}")))?;
    Ok(builder.schema().as_ref().clone())
}

pub async fn read_parquet_batches(reader: Reader, batch_size: usize) -> QueryOutput {
    let async_reader = OpendalAsyncReader::new(reader);
    let stream = ParquetRecordBatchStreamBuilder::new(async_reader)
        .await
        .map_err(|e| ConnectorError::IOError(format!("Failed to open Parquet reader: {e}")))?
        .with_batch_size(batch_size)
        .build()
        .map_err(|e| ConnectorError::IOError(format!("Failed to build Parquet reader: {e}")))?;

    Ok(Box::pin(stream.map(|batch| {
        batch.map_err(|e| ConnectorError::IOError(format!("Parquet read error: {e}")))
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field};
    use arrow::record_batch::RecordBatch;
    use futures::TryStreamExt;
    use opendal::{Operator, services::Fs, services::Memory};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    async fn memory_reader(data: &[u8]) -> Reader {
        let op = Operator::new(Memory::default()).unwrap();
        op.write("test.parquet", data.to_vec()).await.unwrap();
        op.reader("test.parquet").await.unwrap()
    }

    #[tokio::test]
    async fn test_parquet_roundtrip() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec![Some("a"), Some("b"), None])),
            ],
        )
        .unwrap();

        let mut buf = Vec::new();
        {
            let mut writer = parquet::arrow::ArrowWriter::try_new(&mut buf, schema.clone(), None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }

        let reader = memory_reader(&buf).await;
        let read_schema = read_parquet_schema(reader).await.unwrap();
        assert_eq!(read_schema.fields().len(), 2);
        assert_eq!(read_schema.field(0).name(), "id");
        assert_eq!(read_schema.field(1).name(), "name");

        let data = Bytes::from(buf);
        let reader = ParquetRecordBatchReaderBuilder::try_new(data)
            .unwrap()
            .with_batch_size(1024)
            .build()
            .unwrap();
        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 3);
    }

    #[test]
    fn test_parquet_batch_size() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));

        let ids: Vec<i32> = (0..100).collect();
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int32Array::from(ids))]).unwrap();

        let mut buf = Vec::new();
        {
            let mut writer = parquet::arrow::ArrowWriter::try_new(&mut buf, schema.clone(), None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }
        let data = Bytes::from(buf);

        let reader = ParquetRecordBatchReaderBuilder::try_new(data)
            .unwrap()
            .with_batch_size(30)
            .build()
            .unwrap();
        let batches: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 100);
        assert!(batches.len() >= 3);
    }

    fn testdata_reader(filename: &str) -> Reader {
        let testdata_dir = format!("{}/testdata", env!("CARGO_MANIFEST_DIR"));
        let op = Operator::new(Fs::default().root(&testdata_dir)).unwrap();
        futures::executor::block_on(op.reader(filename)).unwrap()
    }

    #[tokio::test]
    async fn test_parquet_schema_from_file() {
        let reader = testdata_reader("sample.parquet");
        let schema = read_parquet_schema(reader).await.unwrap();
        assert_eq!(schema.fields().len(), 4);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(schema.field(2).name(), "score");
        assert_eq!(schema.field(3).name(), "active");
    }

    #[tokio::test]
    async fn test_parquet_batches_from_file() {
        let reader = testdata_reader("sample.parquet");
        let batches: Vec<_> = read_parquet_batches(reader, 1024)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 5);
    }
}
