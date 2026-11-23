use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::header::HeaderMap;
use actix_web::middleware::Next;
use actix_web::{HttpMessage, HttpResponse, ResponseError, web};
use std::time::Instant;

use super::access_log::{AuditError, AuditQuery, RequestId, log_request, resource_action_for};
use super::endpoints::ApiContext;
use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use commons::api::{X_REMOTE_GROUPS, X_REMOTE_USER, X_TENANT_ID};
use commons::utils::audit::AuditConfig;
use opentelemetry::propagation::Extractor;
use tracing::Instrument;
use tracing::field::Empty;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

fn error_response(err: EndpointError) -> HttpResponse {
    let error: RestErrorResponse = err.into();
    error.error_response()
}

/// HeaderExtractor adapts an actix `HeaderMap` to the OpenTelemetry
/// `Extractor` trait so the configured propagator can read `traceparent` and
/// `tracestate` off an incoming request.
struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|name| name.as_str()).collect()
    }
}

/// utf8_header returns a header value decoded as UTF-8. kube-rbac-proxy
/// writes identity names as raw UTF-8, which `HeaderValue::to_str` rejects as
/// non-ASCII, so identity headers are decoded from their bytes instead.
fn utf8_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    std::str::from_utf8(headers.get(name)?.as_bytes()).ok()
}

/// trace_request records a server span for every request entering the scope it
/// wraps. When the caller sends a valid `traceparent` header the span joins
/// that trace as a child; otherwise it starts a new trace. Spans are only
/// exported when a `[trace] exporter` is configured, but the span is always
/// recorded so request fields show up in the logs.
pub async fn trace_request(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    let started = Instant::now();
    let method = req.method().clone();
    let route = req.match_pattern().unwrap_or_else(|| req.path().to_string());
    let audit_config = req
        .app_data::<web::Data<AuditConfig>>()
        .map(|config| *config.get_ref())
        .unwrap_or_default();
    let audit_enabled = audit_config.enabled;
    let audit_operation = resource_action_for(&method, &route);
    let request_id = Uuid::new_v4().to_string();
    let tenant_id = HeaderExtractor(req.headers())
        .get(X_TENANT_ID)
        .unwrap_or_default()
        .to_string();
    // Decode the audit actor the same way validate_headers decodes the
    // identity, so an authorized non-ASCII user is not logged as anonymous.
    let actor = utf8_header(req.headers(), X_REMOTE_USER)
        .filter(|actor| !actor.is_empty())
        .unwrap_or("anonymous")
        .to_string();
    req.extensions_mut().insert(RequestId(request_id.clone()));

    let span = tracing::info_span!(
        "http_request",
        otel.name = %format!("{method} {route}"),
        otel.kind = "server",
        otel.status_code = Empty,
        http.request.method = %method,
        http.route = %route,
        http.response.status_code = Empty,
        request_id = %request_id,
        request.path = %req.path(),
        url.path = %req.path(),
        tenant.id = %tenant_id,
    );

    let parent = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(req.headers()))
    });
    // An empty context makes the span a new root, which is what we want when the
    // caller sent no `traceparent`. Every failure mode here is routine rather
    // than exceptional: no OpenTelemetry layer is installed when no exporter is
    // configured, and the span is absent when filtered out by `RUST_LOG`.
    let _ = span.set_parent(parent);

    let response = next.call(req).instrument(span.clone()).await;

    // A handler can record what the request actually read (for example the
    // path of a binary download) in the request extensions as `AuditQuery`,
    // so the audit event can carry it alongside the connection ID. Route
    // parameters are also only available after routing has dispatched the
    // request to a handler, so the connection ID is read from the returned
    // request rather than the incoming one.
    let query = response
        .as_ref()
        .ok()
        .and_then(|res| res.request().extensions().get::<AuditQuery>().cloned())
        .map(|query| query.0);
    let resource_id = response
        .as_ref()
        .ok()
        .and_then(|res| res.request().match_info().get("id").map(ToOwned::to_owned));

    match &response {
        Ok(res) => {
            let status = res.status();
            span.record("http.response.status_code", status.as_u16());
            if status.is_server_error() {
                span.record("otel.status_code", "ERROR");
            }
        },
        Err(err) => {
            span.record("http.response.status_code", err.error_response().status().as_u16());
            span.record("otel.status_code", "ERROR");
        },
    }

    if audit_enabled && let Some((resource, action)) = audit_operation {
        let (status_code, error) = match &response {
            Ok(res) => {
                let status = res.status();
                let error = res.response().extensions().get::<AuditError>().cloned().or_else(|| {
                    (!status.is_success()).then(|| AuditError {
                        code: status.as_str().to_string(),
                        message: status.canonical_reason().unwrap_or("Request failed").to_string(),
                    })
                });
                (status.as_u16(), error)
            },
            Err(err) => {
                let response = err.error_response();
                let status = response.status();
                let error = response
                    .extensions()
                    .get::<AuditError>()
                    .cloned()
                    .unwrap_or_else(|| AuditError {
                        code: status.as_str().to_string(),
                        message: status.canonical_reason().unwrap_or("Request failed").to_string(),
                    });
                (status.as_u16(), Some(error))
            },
        };
        let _entered = span.enter();
        log_request(
            audit_config,
            &request_id,
            &tenant_id,
            &actor,
            resource,
            action,
            query.as_deref(),
            resource_id.as_deref(),
            status_code,
            error.as_ref(),
            started.elapsed(),
        );
    }

    response
}

pub async fn validate_headers(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    let tenant_id = match req.headers().get(X_TENANT_ID) {
        Some(value) => match value.to_str() {
            Ok(v) if !v.is_empty() => v.to_string(),
            Ok(_) => {
                return Ok(req
                    .into_response(error_response(EndpointError::InvalidHeaderValue(
                        X_TENANT_ID.to_string(),
                    )))
                    .map_into_right_body());
            },
            Err(_) => {
                return Ok(req
                    .into_response(error_response(EndpointError::InvalidHeaderValue(
                        X_TENANT_ID.to_string(),
                    )))
                    .map_into_right_body());
            },
        },
        None => {
            return Ok(req
                .into_response(error_response(EndpointError::HeaderNotFound(X_TENANT_ID.to_string())))
                .map_into_right_body());
        },
    };

    // kube-rbac-proxy authenticates the caller and strips the Authorization
    // header before forwarding, so no bearer token ever reaches the service:
    // identity arrives solely as the proxy-injected x-remote-user and
    // x-remote-groups. See crate::auth::resolve_identity for why the groups
    // header is trusted despite its delimiter-joined encoding. Decode with
    // utf8_header: otherwise one non-ASCII group name would drop the whole
    // groups list.
    let header = |name: &str| {
        let decoded = utf8_header(req.headers(), name);
        if decoded.is_none() && req.headers().contains_key(name) {
            tracing::warn!(header = name, "ignoring identity header that is not valid UTF-8");
        }
        decoded
    };
    let identity = crate::auth::resolve_identity(header(X_REMOTE_USER), header(X_REMOTE_GROUPS));

    let request_id = req
        .extensions()
        .get::<RequestId>()
        .map(|request_id| request_id.0.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    req.extensions_mut().insert(ApiContext {
        tenant_id,
        identity,
        request_id,
    });

    next.call(req).await.map(ServiceResponse::map_into_left_body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header::{HeaderName, HeaderValue};
    use actix_web::{App, test as awtest};
    use opentelemetry::propagation::TextMapPropagator;
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN_ID: &str = "00f067aa0ba902b7";

    #[derive(Clone, Default)]
    struct AuditCapture(Arc<Mutex<Vec<HashMap<String, String>>>>);

    impl<S> Layer<S> for AuditCapture
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
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

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn test_header_extractor_reads_values_and_keys() {
        let map = headers(&[("traceparent", "abc"), (X_TENANT_ID, "test-tenant")]);
        let extractor = HeaderExtractor(&map);

        assert_eq!(extractor.get("traceparent"), Some("abc"));
        assert_eq!(extractor.get(X_TENANT_ID), Some("test-tenant"));
        assert_eq!(extractor.get("missing"), None);

        let mut keys = extractor.keys();
        keys.sort_unstable();
        assert_eq!(keys, vec!["traceparent", X_TENANT_ID]);
    }

    #[test]
    fn test_extract_joins_incoming_trace() {
        let map = headers(&[("traceparent", &format!("00-{TRACE_ID}-{SPAN_ID}-01"))]);
        let parent = TraceContextPropagator::new().extract(&HeaderExtractor(&map));

        let span_context = parent.span().span_context().clone();
        assert!(span_context.is_valid());
        assert!(span_context.is_remote());
        assert_eq!(span_context.trace_id().to_string(), TRACE_ID);
        assert_eq!(span_context.span_id().to_string(), SPAN_ID);
        assert!(span_context.is_sampled());
    }

    #[test]
    fn test_extract_without_traceparent_starts_new_trace() {
        let map = headers(&[(X_TENANT_ID, "test-tenant")]);
        let parent = TraceContextPropagator::new().extract(&HeaderExtractor(&map));

        assert!(!parent.span().span_context().is_valid());
    }

    #[test]
    fn test_extract_with_malformed_traceparent_starts_new_trace() {
        let map = headers(&[("traceparent", "not-a-traceparent")]);
        let parent = TraceContextPropagator::new().extract(&HeaderExtractor(&map));

        assert!(!parent.span().span_context().is_valid());
    }

    #[actix_web::test]
    async fn test_audit_enabled_emits_one_structured_event() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let app = awtest::init_service(
                App::new()
                    .app_data(web::Data::new(AuditConfig {
                        enabled: true,
                        log_queries: false,
                    }))
                    .service(
                        web::scope("/api/v1alpha1/data")
                            .wrap(actix_web::middleware::from_fn(validate_headers))
                            .wrap(actix_web::middleware::from_fn(trace_request))
                            .route("/connections", web::get().to(|| async { HttpResponse::Ok().finish() })),
                    ),
            )
            .await;
            let request = awtest::TestRequest::get()
                .uri("/api/v1alpha1/data/connections")
                .insert_header((X_TENANT_ID, "tenant"))
                .insert_header((X_REMOTE_USER, "alice"))
                .to_request();

            let response = awtest::call_service(&app, request).await;
            assert_eq!(response.status(), 200);
        }
        .with_subscriber(subscriber)
        .await;

        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("audit.tenant_id").map(String::as_str), Some("tenant"));
        assert_eq!(events[0].get("audit.actor").map(String::as_str), Some("alice"));
        assert_eq!(events[0].get("audit.resource").map(String::as_str), Some("connection"));
        assert_eq!(events[0].get("audit.action").map(String::as_str), Some("list"));
        assert!(!events[0].contains_key("method"));
        assert!(!events[0].contains_key("route"));
        assert!(!events[0].contains_key("outcome"));
        assert_eq!(events[0].get("status_code").map(String::as_str), Some("200"));
        assert!(
            events[0]
                .get("request_id")
                .is_some_and(|value| Uuid::parse_str(value).is_ok())
        );
    }

    #[actix_web::test]
    async fn test_audit_records_non_ascii_actor() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let app = awtest::init_service(
                App::new()
                    .app_data(web::Data::new(AuditConfig {
                        enabled: true,
                        log_queries: false,
                    }))
                    .service(
                        web::scope("/api/v1alpha1/data")
                            .wrap(actix_web::middleware::from_fn(validate_headers))
                            .wrap(actix_web::middleware::from_fn(trace_request))
                            .route("/connections", web::get().to(|| async { HttpResponse::Ok().finish() })),
                    ),
            )
            .await;
            let user = actix_web::http::header::HeaderValue::from_bytes("张三".as_bytes()).unwrap();
            let request = awtest::TestRequest::get()
                .uri("/api/v1alpha1/data/connections")
                .insert_header((X_TENANT_ID, "tenant"))
                .insert_header((X_REMOTE_USER, user))
                .to_request();

            let response = awtest::call_service(&app, request).await;
            assert_eq!(response.status(), 200);
        }
        .with_subscriber(subscriber)
        .await;

        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("audit.actor").map(String::as_str), Some("张三"));
    }

    #[actix_web::test]
    async fn test_audit_download_records_query_path() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let app = awtest::init_service(
                App::new()
                    .app_data(web::Data::new(AuditConfig {
                        enabled: true,
                        log_queries: true,
                    }))
                    .service(
                        web::scope("/api/v1alpha1/data")
                            .wrap(actix_web::middleware::from_fn(validate_headers))
                            .wrap(actix_web::middleware::from_fn(trace_request))
                            .route(
                                "/connections/{id}/binary",
                                web::get().to(|req: actix_web::HttpRequest| async move {
                                    req.extensions_mut().insert(AuditQuery("reports/q3.csv".to_string()));
                                    HttpResponse::Ok().finish()
                                }),
                            ),
                    ),
            )
            .await;
            let request = awtest::TestRequest::get()
                .uri("/api/v1alpha1/data/connections/conn-1/binary?path=reports%2Fq3.csv")
                .insert_header((X_TENANT_ID, "tenant"))
                .insert_header((X_REMOTE_USER, "alice"))
                .to_request();

            let response = awtest::call_service(&app, request).await;
            assert_eq!(response.status(), 200);
        }
        .with_subscriber(subscriber)
        .await;

        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("audit.resource").map(String::as_str), Some("connection"));
        assert_eq!(events[0].get("audit.action").map(String::as_str), Some("download"));
        assert_eq!(events[0].get("audit.query").map(String::as_str), Some("reports/q3.csv"));
        assert_eq!(events[0].get("resource_id").map(String::as_str), Some("conn-1"));
    }

    #[actix_web::test]
    async fn test_audit_download_omits_query_when_log_queries_disabled() {
        let capture = AuditCapture::default();
        let subscriber = Registry::default().with(capture.clone());

        async {
            let app = awtest::init_service(
                App::new()
                    .app_data(web::Data::new(AuditConfig {
                        enabled: true,
                        log_queries: false,
                    }))
                    .service(
                        web::scope("/api/v1alpha1/data")
                            .wrap(actix_web::middleware::from_fn(validate_headers))
                            .wrap(actix_web::middleware::from_fn(trace_request))
                            .route(
                                "/connections/{id}/binary",
                                web::get().to(|req: actix_web::HttpRequest| async move {
                                    req.extensions_mut().insert(AuditQuery("reports/q3.csv".to_string()));
                                    HttpResponse::Ok().finish()
                                }),
                            ),
                    ),
            )
            .await;
            let request = awtest::TestRequest::get()
                .uri("/api/v1alpha1/data/connections/conn-1/binary?path=reports%2Fq3.csv")
                .insert_header((X_TENANT_ID, "tenant"))
                .insert_header((X_REMOTE_USER, "alice"))
                .to_request();

            let response = awtest::call_service(&app, request).await;
            assert_eq!(response.status(), 200);
        }
        .with_subscriber(subscriber)
        .await;

        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get("audit.resource").map(String::as_str), Some("connection"));
        assert_eq!(events[0].get("audit.action").map(String::as_str), Some("download"));
        assert!(!events[0].contains_key("audit.query"));
        assert_eq!(events[0].get("resource_id").map(String::as_str), Some("conn-1"));
    }
}
