use commons::api::X_TENANT_ID;
use futures::Stream;
use opentelemetry::propagation::Extractor;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::Instrument;
use tracing::field::Empty;
use tracing_opentelemetry::OpenTelemetrySpanExt;

const HEALTH_PATH_PREFIX: &str = "/grpc.health.v1.Health/";
const GRPC_STATUS: &str = "grpc-status";

/// MetadataExtractor adapts the HTTP headers that carry gRPC metadata to the
/// OpenTelemetry `Extractor` trait, so the configured propagator can read
/// `traceparent` and `tracestate` off an incoming request.
struct MetadataExtractor<'a>(&'a http::HeaderMap);

impl Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|name| name.as_str()).collect()
    }
}

/// split_grpc_path splits a gRPC request path of the form
/// `/package.Service/Method` into its service and method halves.
fn split_grpc_path(path: &str) -> (&str, &str) {
    let trimmed = path.trim_start_matches('/');
    match trimmed.rsplit_once('/') {
        Some((service, method)) => (service, method),
        None => (trimmed, ""),
    }
}

/// TraceLayer records a server span for every gRPC call, parented to the
/// caller's trace when the request carries a `traceparent` header.
#[derive(Clone, Default)]
pub struct TraceLayer;

impl TraceLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for TraceLayer {
    type Service = TraceMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceMiddleware { inner }
    }
}

#[derive(Clone)]
pub struct TraceMiddleware<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for TraceMiddleware<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let path = req.uri().path().to_string();
        if path.starts_with(HEALTH_PATH_PREFIX) {
            return Box::pin(async move { inner.call(req).await });
        }

        let (service, method) = split_grpc_path(&path);
        let span = tracing::info_span!(
            "grpc_request",
            otel.name = %path.trim_start_matches('/'),
            otel.kind = "server",
            otel.status_code = Empty,
            rpc.system = "grpc",
            rpc.service = %service,
            rpc.method = %method,
            rpc.grpc.status_code = Empty,
            tenant.id = MetadataExtractor(req.headers()).get(X_TENANT_ID).unwrap_or_default(),
        );

        let parent = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&MetadataExtractor(req.headers()))
        });
        // An empty context makes the span a new root, which is what we want when
        // the caller sent no `traceparent`.
        let _ = span.set_parent(parent);

        Box::pin(async move {
            let response = inner.call(req).instrument(span.clone()).await;

            match &response {
                // A `grpc-status` header is only present on trailers-only
                // responses, which is how tonic reports an error before any
                // message is streamed. Successful streaming calls carry their
                // status in the trailers instead, which this layer cannot see.
                Ok(res) => {
                    if let Some(code) = res
                        .headers()
                        .get(GRPC_STATUS)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<i32>().ok())
                    {
                        span.record("rpc.grpc.status_code", code);
                        if code != 0 {
                            span.record("otel.status_code", "ERROR");
                        }
                    }
                },
                Err(_) => {
                    span.record("otel.status_code", "ERROR");
                },
            }

            response
        })
    }
}

/// TracedStream keeps a span open until the wrapped stream terminates or is
/// dropped, recording how many batches it produced and whether it failed
/// part-way through. A streaming RPC returns its response head as soon as the
/// handler is done, so without this the request span closes before any data is
/// sent and the transfer itself goes unmeasured.
pub struct TracedStream<T, E> {
    inner: Pin<Box<dyn Stream<Item = Result<T, E>> + Send>>,
    span: tracing::Span,
    batches: u64,
}

impl<T, E> TracedStream<T, E> {
    /// new wraps `inner` in a span describing the Flight operation it serves.
    /// It must be called while the handler's span is current, so the stream
    /// span is recorded as its child.
    pub fn new<S>(operation: &'static str, inner: S) -> Self
    where
        S: Stream<Item = Result<T, E>> + Send + 'static,
    {
        Self {
            inner: Box::pin(inner),
            span: tracing::info_span!(
                "do_get_stream",
                flight.operation = operation,
                otel.status_code = Empty,
                batch_count = Empty,
            ),
            batches: 0,
        }
    }
}

impl<T, E: std::fmt::Display> Stream for TracedStream<T, E> {
    type Item = Result<T, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let _guard = this.span.enter();
        let polled = this.inner.as_mut().poll_next(cx);

        match &polled {
            Poll::Ready(Some(Ok(_))) => this.batches += 1,
            Poll::Ready(Some(Err(e))) => {
                // The error is reported here rather than left to the caller
                // because tonic turns it into trailers, which the TraceLayer
                // cannot see.
                tracing::error!(error = %e, "streaming response failed");
                this.span.record("otel.status_code", "ERROR");
                this.span.record("batch_count", this.batches);
            },
            Poll::Ready(None) => {
                this.span.record("batch_count", this.batches);
            },
            Poll::Pending => {},
        }

        polled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use http::HeaderMap;
    use http::header::HeaderValue;

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN_ID: &str = "00f067aa0ba902b7";

    #[test]
    fn test_split_grpc_path_service_and_method() {
        let (service, method) = split_grpc_path("/arrow.flight.protocol.FlightService/DoGet");
        assert_eq!(service, "arrow.flight.protocol.FlightService");
        assert_eq!(method, "DoGet");
    }

    #[test]
    fn test_split_grpc_path_without_method() {
        let (service, method) = split_grpc_path("/health");
        assert_eq!(service, "health");
        assert_eq!(method, "");
    }

    #[test]
    fn test_metadata_extractor_reads_values_and_keys() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            HeaderValue::from_str(&format!("00-{TRACE_ID}-{SPAN_ID}-01")).unwrap(),
        );
        headers.insert(X_TENANT_ID, HeaderValue::from_static("awc"));

        let extractor = MetadataExtractor(&headers);
        assert_eq!(extractor.get(X_TENANT_ID), Some("awc"));
        assert_eq!(
            extractor.get("traceparent"),
            Some(format!("00-{TRACE_ID}-{SPAN_ID}-01").as_str())
        );
        assert_eq!(extractor.get("missing"), None);

        let keys = extractor.keys();
        assert!(keys.contains(&"traceparent"));
        assert!(keys.contains(&X_TENANT_ID));
    }

    #[test]
    fn test_metadata_extractor_skips_non_ascii_values() {
        let mut headers = HeaderMap::new();
        headers.insert("traceparent", HeaderValue::from_bytes(b"\xff\xfe").unwrap());

        assert_eq!(MetadataExtractor(&headers).get("traceparent"), None);
    }

    fn traced<T: Send + 'static>(items: Vec<Result<T, String>>) -> TracedStream<T, String> {
        TracedStream::new("test", futures::stream::iter(items))
    }

    #[tokio::test]
    async fn test_traced_stream_passes_items_through_and_counts_them() {
        let mut stream = traced(vec![Ok(1), Ok(2), Ok(3)]);

        assert_eq!(stream.next().await, Some(Ok(1)));
        assert_eq!(stream.next().await, Some(Ok(2)));
        assert_eq!(stream.next().await, Some(Ok(3)));
        assert_eq!(stream.next().await, None);
        assert_eq!(stream.batches, 3);
    }

    #[tokio::test]
    async fn test_traced_stream_passes_errors_through() {
        let mut stream = traced(vec![Ok(1), Err("boom".to_string())]);

        assert_eq!(stream.next().await, Some(Ok(1)));
        assert_eq!(stream.next().await, Some(Err("boom".to_string())));
        // The batch that failed is not counted as delivered.
        assert_eq!(stream.batches, 1);
    }

    #[tokio::test]
    async fn test_traced_stream_handles_empty_stream() {
        let mut stream = traced(Vec::<Result<u8, String>>::new());

        assert_eq!(stream.next().await, None);
        assert_eq!(stream.batches, 0);
    }
}
