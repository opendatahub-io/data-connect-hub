use std::ops::Range;
use std::sync::Arc;

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::{FutureExt, TryStreamExt};
use oci_client::client::BlobResponse;
use oci_client::manifest::OciDescriptor;
use oci_client::{Client, Reference};
use parquet::arrow::async_reader::{AsyncFileReader, MetadataSuffixFetch};
use parquet::file::metadata::ParquetMetaData;

pub struct OciAsyncReader {
    client: Client,
    reference: Reference,
    layer: OciDescriptor,
    size: u64,
}

impl OciAsyncReader {
    pub fn new(client: Client, reference: Reference, layer: OciDescriptor) -> Self {
        let size = layer.size as u64;
        Self {
            client,
            reference,
            layer,
            size,
        }
    }

    async fn fetch_range(&self, offset: u64, length: u64) -> Result<Bytes, parquet::errors::ParquetError> {
        let response = self
            .client
            .pull_blob_stream_partial(&self.reference, &self.layer, offset, Some(length))
            .await
            .map_err(|e| parquet::errors::ParquetError::External(Box::new(e)))?;

        let stream = match response {
            BlobResponse::Full(s) => s,
            BlobResponse::Partial(s) => s,
        };

        let mut result = Vec::new();
        let mut byte_stream = stream.stream;
        while let Some(chunk) = byte_stream
            .try_next()
            .await
            .map_err(|e| parquet::errors::ParquetError::External(Box::new(e)))?
        {
            result.extend_from_slice(&chunk);
        }
        Ok(Bytes::from(result))
    }
}

impl AsyncFileReader for OciAsyncReader {
    fn get_bytes(&mut self, range: Range<u64>) -> BoxFuture<'_, parquet::errors::Result<Bytes>> {
        let offset = range.start;
        let length = range.end - range.start;
        async move { self.fetch_range(offset, length).await }.boxed()
    }

    fn get_byte_ranges(&mut self, ranges: Vec<Range<u64>>) -> BoxFuture<'_, parquet::errors::Result<Vec<Bytes>>> {
        async move {
            let mut results = Vec::with_capacity(ranges.len());
            for range in ranges {
                let offset = range.start;
                let length = range.end - range.start;
                results.push(self.fetch_range(offset, length).await?);
            }
            Ok(results)
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

impl MetadataSuffixFetch for &mut OciAsyncReader {
    fn fetch_suffix(&mut self, suffix: usize) -> BoxFuture<'_, parquet::errors::Result<Bytes>> {
        let offset = self.size.saturating_sub(suffix as u64);
        let length = suffix as u64;
        async move { self.fetch_range(offset, length).await }.boxed()
    }
}
