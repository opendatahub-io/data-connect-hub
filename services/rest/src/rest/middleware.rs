use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::header::HeaderMap;
use actix_web::middleware::Next;
use actix_web::{HttpMessage, HttpResponse};

use super::endpoints::ApiContext;
use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use commons::api::X_TENANT_ID;
use opentelemetry::propagation::Extractor;
use tracing::Instrument;
use tracing::field::Empty;
use tracing_opentelemetry::OpenTelemetrySpanExt;

fn error_response(err: EndpointError) -> HttpResponse {
    let error: RestErrorResponse = err.into();
    HttpResponse::BadRequest().json(&error)
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

/// trace_request records a server span for every request entering the scope it
/// wraps. When the caller sends a valid `traceparent` header the span joins
/// that trace as a child; otherwise it starts a new trace. Spans are only
/// exported when a `[trace] exporter` is configured, but the span is always
/// recorded so request fields show up in the logs.
pub async fn trace_request(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    let method = req.method().clone();
    let route = req.match_pattern().unwrap_or_else(|| req.path().to_string());

    let span = tracing::info_span!(
        "http_request",
        otel.name = %format!("{method} {route}"),
        otel.kind = "server",
        otel.status_code = Empty,
        http.request.method = %method,
        http.route = %route,
        http.response.status_code = Empty,
        url.path = %req.path(),
        tenant.id = HeaderExtractor(req.headers()).get(X_TENANT_ID).unwrap_or_default(),
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

    req.extensions_mut().insert(ApiContext { tenant_id });

    next.call(req).await.map(ServiceResponse::map_into_left_body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header::{HeaderName, HeaderValue};
    use opentelemetry::propagation::TextMapPropagator;
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry_sdk::propagation::TraceContextPropagator;

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN_ID: &str = "00f067aa0ba902b7";

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
}
