use crate::flight::audit::{
    AuditConfig, AuditContext, PendingAudit, RpcAuditState, RpcRequestId, do_get_operation, log_rpc,
};
use commons::api::{X_DATA_CONNECTION_ID, X_TENANT_ID};
use futures::Stream;
use opentelemetry::propagation::Extractor;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tonic::Status;
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
pub struct TraceLayer {
    audit: AuditConfig,
}

impl TraceLayer {
    pub fn new(audit: AuditConfig) -> Self {
        Self { audit }
    }
}

impl<S> Layer<S> for TraceLayer {
    type Service = TraceMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceMiddleware {
            inner,
            audit: self.audit,
        }
    }
}

#[derive(Clone)]
pub struct TraceMiddleware<S> {
    inner: S,
    audit: AuditConfig,
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

    fn call(&mut self, mut req: http::Request<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let audit = self.audit;
        let started = Instant::now();

        let path = req.uri().path().to_string();
        if path.starts_with(HEALTH_PATH_PREFIX) {
            return Box::pin(async move { inner.call(req).await });
        }

        let (service, method) = split_grpc_path(&path);
        let request_id = uuid::Uuid::new_v4().to_string();
        let audit_state = Arc::new(RpcAuditState::default());
        req.extensions_mut().insert(RpcRequestId(request_id.clone()));
        req.extensions_mut().insert(audit_state.clone());
        let tenant_id = MetadataExtractor(req.headers())
            .get(X_TENANT_ID)
            .unwrap_or_default()
            .to_string();
        let resource_id = MetadataExtractor(req.headers())
            .get(X_DATA_CONNECTION_ID)
            .map(ToOwned::to_owned);
        let span = tracing::info_span!(
            "grpc_request",
            otel.name = %path.trim_start_matches('/'),
            otel.kind = "server",
            otel.status_code = Empty,
            request.path = %path,
            rpc.system = "grpc",
            rpc.service = %service,
            rpc.method = %method,
            rpc.grpc.status_code = Empty,
            request_id = %request_id,
            tenant.id = %tenant_id,
        );

        let parent = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&MetadataExtractor(req.headers()))
        });
        // An empty context makes the span a new root, which is what we want when
        // the caller sent no `traceparent`.
        let _ = span.set_parent(parent);

        Box::pin(async move {
            let (dispatch_resource, dispatch_action) = match path.as_str() {
                "/arrow.flight.protocol.FlightService/DoGet" => ("flight_server", "do_get"),
                "/arrow.flight.protocol.FlightService/DoAction" => ("flight_server", "do_action"),
                _ => ("flight_server", "dispatch"),
            };
            let dispatch_context = AuditContext::from_http(
                request_id.clone(),
                tenant_id.clone(),
                resource_id.clone(),
                audit_state.clone(),
            );
            let mut dispatch_audit = PendingAudit::new(
                audit.enabled
                    && matches!(
                        path.as_str(),
                        "/arrow.flight.protocol.FlightService/DoGet" | "/arrow.flight.protocol.FlightService/DoAction"
                    ),
                dispatch_context,
                dispatch_resource,
                dispatch_action,
                started,
            );
            let response = inner.call(req).instrument(span.clone()).await;
            dispatch_audit.transfer_to_stream();

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
                            audit_dispatch_failure(
                                audit,
                                &path,
                                &request_id,
                                &tenant_id,
                                resource_id.clone(),
                                audit_state.clone(),
                                tonic::Code::from_i32(code),
                                started.elapsed(),
                                &span,
                            );
                        }
                    }
                },
                Err(_) => {
                    span.record("otel.status_code", "ERROR");
                    audit_dispatch_failure(
                        audit,
                        &path,
                        &request_id,
                        &tenant_id,
                        resource_id,
                        audit_state,
                        tonic::Code::Unknown,
                        started.elapsed(),
                        &span,
                    );
                },
            }

            response
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_dispatch_failure(
    audit: AuditConfig,
    path: &str,
    request_id: &str,
    tenant_id: &str,
    resource_id: Option<String>,
    state: Arc<RpcAuditState>,
    code: tonic::Code,
    duration: Duration,
    span: &tracing::Span,
) {
    let (resource, action) = match path {
        "/arrow.flight.protocol.FlightService/DoGet" => ("flight_server", "do_get"),
        "/arrow.flight.protocol.FlightService/DoAction" => ("flight_server", "do_action"),
        _ => return,
    };
    let status = Status::new(code, "gRPC dispatch failed");
    let context = AuditContext::from_http(request_id.to_string(), tenant_id.to_string(), resource_id, state);
    let _guard = span.enter();
    log_rpc(audit.enabled, &context, resource, action, Some(&status), duration);
}

/// TracedStream keeps a span open until the wrapped stream terminates or is
/// dropped, recording how many batches it produced and whether it failed
/// part-way through. A streaming RPC returns its response head as soon as the
/// handler is done, so without this the request span closes before any data is
/// sent and the transfer itself goes unmeasured.
pub struct TracedStream<T> {
    inner: Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>,
    span: tracing::Span,
    batches: u64,
    audit: Option<StreamAudit>,
    finished: bool,
}

pub(crate) struct StreamAudit {
    enabled: bool,
    context: AuditContext,
    resource: &'static str,
    action: &'static str,
    started: Instant,
}

impl StreamAudit {
    pub(crate) fn new(
        enabled: bool,
        context: AuditContext,
        resource: &'static str,
        action: &'static str,
        started: Instant,
    ) -> Self {
        Self {
            enabled,
            context,
            resource,
            action,
            started,
        }
    }
}

impl<T> TracedStream<T> {
    /// new wraps `inner` in a span describing the Flight operation it serves.
    /// It must be called while the handler's span is current, so the stream
    /// span is recorded as its child.
    pub fn new<S>(operation: &'static str, inner: S) -> Self
    where
        S: Stream<Item = Result<T, Status>> + Send + 'static,
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
            audit: None,
            finished: false,
        }
    }

    pub(crate) fn new_audited<S>(
        operation: &'static str,
        inner: S,
        enabled: bool,
        context: AuditContext,
        started: Instant,
    ) -> Self
    where
        S: Stream<Item = Result<T, Status>> + Send + 'static,
    {
        let (resource, action) = do_get_operation(operation);
        Self::new_audited_operation(
            operation,
            inner,
            StreamAudit::new(enabled, context, resource, action, started),
        )
    }

    pub(crate) fn new_audited_operation<S>(operation: &'static str, inner: S, audit: StreamAudit) -> Self
    where
        S: Stream<Item = Result<T, Status>> + Send + 'static,
    {
        let span = tracing::info_span!(
            "do_get_stream",
            flight.operation = operation,
            otel.status_code = Empty,
            batch_count = Empty,
            request_id = %audit.context.request_id,
        );
        Self {
            inner: Box::pin(inner),
            span,
            batches: 0,
            audit: Some(audit),
            finished: false,
        }
    }

    fn finish(&mut self, status: Option<&Status>) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.span.record("batch_count", self.batches);
        if status.is_some() {
            self.span.record("otel.status_code", "ERROR");
        }
        if let Some(audit) = &self.audit {
            let _guard = self.span.enter();
            log_rpc(
                audit.enabled,
                &audit.context,
                audit.resource,
                audit.action,
                status,
                audit.started.elapsed(),
            );
        }
    }
}

impl<T> Stream for TracedStream<T> {
    type Item = Result<T, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let span = this.span.clone();
        let _guard = span.enter();
        let polled = this.inner.as_mut().poll_next(cx);

        match &polled {
            Poll::Ready(Some(Ok(_))) => this.batches += 1,
            Poll::Ready(Some(Err(e))) => {
                // The error is reported here rather than left to the caller
                // because tonic turns it into trailers, which the TraceLayer
                // cannot see.
                tracing::error!(error = %e, "streaming response failed");
                let status = e.clone();
                this.finish(Some(&status));
            },
            Poll::Ready(None) => {
                this.finish(None);
            },
            Poll::Pending => {},
        }

        polled
    }
}

impl<T> Drop for TracedStream<T> {
    fn drop(&mut self) {
        if !self.finished {
            let status = Status::cancelled("stream cancelled");
            self.finish(Some(&status));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use http::HeaderMap;
    use http::header::HeaderValue;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::{Context as LayerContext, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN_ID: &str = "00f067aa0ba902b7";

    #[derive(Clone, Default)]
    struct AuditCapture(Arc<Mutex<Vec<HashMap<String, String>>>>);

    impl<S> Layer<S> for AuditCapture
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
            if event.metadata().target() != "audit" {
                return;
            }
            let mut fields = HashMap::new();
            event.record(&mut StringVisitor(&mut fields));
            self.0.lock().unwrap().push(fields);
        }
    }

    struct StringVisitor<'a>(&'a mut HashMap<String, String>);

    impl tracing::field::Visit for StringVisitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    fn audit_context() -> AuditContext {
        AuditContext {
            request_id: uuid::Uuid::new_v4().to_string(),
            parent_request_id: Some(uuid::Uuid::new_v4().to_string()),
            actor: "alice".to_string(),
            caller_principal: "rest-service".to_string(),
            tenant_id: "tenant".to_string(),
            resource_id: Some("connection".to_string()),
            query: Some("SELECT 1 AS audit_test".to_string()),
            state: None,
        }
    }

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

    fn traced<T: Send + 'static>(items: Vec<Result<T, Status>>) -> TracedStream<T> {
        TracedStream::new("test", futures::stream::iter(items))
    }

    #[tokio::test]
    async fn test_traced_stream_passes_items_through_and_counts_them() {
        let mut stream = traced(vec![Ok(1), Ok(2), Ok(3)]);

        assert_eq!(stream.next().await.unwrap().unwrap(), 1);
        assert_eq!(stream.next().await.unwrap().unwrap(), 2);
        assert_eq!(stream.next().await.unwrap().unwrap(), 3);
        assert!(stream.next().await.is_none());
        assert_eq!(stream.batches, 3);
    }

    #[tokio::test]
    async fn test_traced_stream_passes_errors_through() {
        let mut stream = traced(vec![Ok(1), Err(Status::internal("boom"))]);

        assert_eq!(stream.next().await.unwrap().unwrap(), 1);
        assert_eq!(stream.next().await.unwrap().unwrap_err().code(), tonic::Code::Internal);
        // The batch that failed is not counted as delivered.
        assert_eq!(stream.batches, 1);
    }

    #[tokio::test]
    async fn test_traced_stream_handles_empty_stream() {
        let mut stream = traced(Vec::<Result<u8, Status>>::new());

        assert!(stream.next().await.is_none());
        assert_eq!(stream.batches, 0);
    }

    #[tokio::test]
    async fn audited_stream_emits_once_on_success() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let mut stream = TracedStream::new_audited(
                "statement",
                futures::stream::iter(vec![Ok(1)]),
                true,
                audit_context(),
                Instant::now(),
            );
            assert_eq!(stream.next().await.unwrap().unwrap(), 1);
            assert!(stream.next().await.is_none());
        }
        .with_subscriber(subscriber)
        .await;

        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("audit.resource").map(String::as_str), Some("connection"));
        assert_eq!(events[0].get("audit.action").map(String::as_str), Some("query"));
        assert_eq!(
            events[0].get("audit.query").map(String::as_str),
            Some("SELECT 1 AS audit_test")
        );
        assert_eq!(events[0].get("grpc_status").map(String::as_str), Some("OK"));
        assert!(!events[0].contains_key("error_code"));
    }

    #[tokio::test]
    async fn audited_stream_emits_once_on_error_and_drop() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let mut stream = TracedStream::<u8>::new_audited(
                "statement",
                futures::stream::iter(vec![Err(Status::internal("sensitive backend error"))]),
                true,
                audit_context(),
                Instant::now(),
            );
            assert_eq!(stream.next().await.unwrap().unwrap_err().code(), tonic::Code::Internal);
            drop(stream);
        }
        .with_subscriber(subscriber)
        .await;

        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("grpc_status").map(String::as_str), Some("Internal"));
        assert_eq!(events[0].get("error_code").map(String::as_str), Some("Internal"));
        assert_eq!(
            events[0].get("error_message").map(String::as_str),
            Some("gRPC request failed")
        );
        assert!(
            !events[0]
                .values()
                .any(|value| value.contains("sensitive backend error"))
        );
    }

    #[tokio::test]
    async fn audited_stream_records_cancellation_on_drop() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let stream = TracedStream::<u8>::new_audited(
                "statement",
                futures::stream::pending(),
                true,
                audit_context(),
                Instant::now(),
            );
            drop(stream);
        }
        .with_subscriber(subscriber)
        .await;

        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("grpc_status").map(String::as_str), Some("Cancelled"));
    }

    #[tokio::test]
    async fn audited_stream_emits_nothing_when_disabled() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let mut stream = TracedStream::new_audited(
                "statement",
                futures::stream::iter(vec![Ok(1)]),
                false,
                audit_context(),
                Instant::now(),
            );
            assert_eq!(stream.next().await.unwrap().unwrap(), 1);
            assert!(stream.next().await.is_none());
        }
        .with_subscriber(subscriber)
        .await;

        assert!(capture.0.lock().unwrap().is_empty());
    }
}
