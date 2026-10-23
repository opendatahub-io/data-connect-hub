pub mod config;

use anyhow::{Context, Result};
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tonic::transport::{Certificate, ClientTlsConfig};
use tracing::info;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::Registry;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::FormattedFields;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, JsonFields, Writer};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

struct RestJsonFormat;

#[derive(Default)]
struct RestJsonVisitor(serde_json::Map<String, serde_json::Value>);

impl tracing::field::Visit for RestJsonVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_owned(), serde_json::Value::String(format!("{value:?}")));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_owned(), serde_json::Value::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_owned(), serde_json::Value::from(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_owned(), serde_json::Value::from(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.0.insert(field.name().to_owned(), serde_json::Value::from(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .insert(field.name().to_owned(), serde_json::Value::String(value.to_owned()));
    }
}

impl<S, N> FormatEvent<S, N> for RestJsonFormat
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let mut visitor = RestJsonVisitor::default();
        event.record(&mut visitor);
        let fields = visitor.0;
        let mut log = serde_json::Map::new();

        let mut timestamp = String::new();
        tracing_subscriber::fmt::time::SystemTime.format_time(&mut Writer::new(&mut timestamp))?;
        log.insert("ts".to_owned(), serde_json::Value::String(timestamp));
        log.insert(
            "level".to_owned(),
            serde_json::Value::String(event.metadata().level().to_string()),
        );
        log.insert(
            "logger".to_owned(),
            serde_json::Value::String(event.metadata().target().to_owned()),
        );
        log.insert(
            "service.name".to_owned(),
            serde_json::Value::String("data-connect-hub".to_owned()),
        );

        let span_context = tracing::Span::current().context().span().span_context().clone();
        if span_context.is_valid() {
            log.insert(
                "trace_id".to_owned(),
                serde_json::Value::String(span_context.trace_id().to_string()),
            );
            log.insert(
                "span_id".to_owned(),
                serde_json::Value::String(span_context.span_id().to_string()),
            );
        }

        if let Some(span) = event
            .parent()
            .and_then(|id| ctx.span(id))
            .or_else(|| ctx.lookup_current())
            && let Some(fields) = span.extensions().get::<FormattedFields<N>>()
            && let Ok(serde_json::Value::Object(fields)) = serde_json::from_str(&fields.fields)
            && let Some(request_path) = fields.get("request.path").and_then(serde_json::Value::as_str)
        {
            log.insert(
                "request-path".to_owned(),
                serde_json::Value::String(request_path.to_owned()),
            );
        }

        if !fields.is_empty() {
            let mut fields = fields;
            let message = fields.remove("message").unwrap_or(serde_json::Value::Null);
            log.insert("msg".to_owned(), message);
            log.extend(fields);
        } else {
            log.insert("msg".to_owned(), serde_json::Value::Null);
        }

        writer.write_str(&serde_json::to_string(&log).map_err(|_| std::fmt::Error)?)?;
        writer.write_char('\n')
    }
}

/// ENV_OTLP_ENDPOINT names the environment variable holding the OTLP/gRPC
/// collector URL. It is set from `spec.trace.exporter` on the DataConnectService.
pub const ENV_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// ENV_OTLP_INSECURE names the environment variable that disables transport
/// security. It is set from `spec.trace.insecure` on the DataConnectService.
pub const ENV_OTLP_INSECURE: &str = "OTEL_EXPORTER_OTLP_INSECURE";

/// ENV_OTLP_CERTIFICATE names the environment variable holding the path to the
/// PEM certificate of the CA that signed the collector's TLS certificate. It is
/// set from `spec.trace.certificate` on the DataConnectService.
pub const ENV_OTLP_CERTIFICATE: &str = "OTEL_EXPORTER_OTLP_CERTIFICATE";

/// TraceConfig is the OTLP exporter configuration, read from the standard
/// OpenTelemetry environment variables that the dc-controller sets on the
/// service container from `spec.trace` of the DataConnectService. Tracing is
/// enabled only when an endpoint is present; otherwise spans are still recorded
/// in-process but never exported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceConfig {
    pub endpoint: Option<String>,
    pub insecure: bool,
    pub certificate: Option<String>,
}

impl TraceConfig {
    /// from_env reads the OTLP exporter settings from the process environment.
    pub fn from_env() -> Self {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let value = |key: &str| get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        TraceConfig {
            endpoint: value(ENV_OTLP_ENDPOINT),
            insecure: value(ENV_OTLP_INSECURE).is_some_and(|v| v.eq_ignore_ascii_case("true")),
            certificate: value(ENV_OTLP_CERTIFICATE),
        }
    }
}

/// log_trace_exporter reports the tracing configuration resolved from the
/// environment, so an operator can tell from the logs whether spans are being
/// exported and where to.
pub fn log_trace_exporter(trace: &TraceConfig) {
    match trace.endpoint.as_deref() {
        Some(endpoint) => info!(
            endpoint,
            insecure = trace.insecure,
            certificate = trace.certificate.as_deref().unwrap_or_default(),
            "Trace exporter configured"
        ),
        None => info!("No trace exporter configured; spans are not exported"),
    }
}

/// resolve_endpoint applies `OTEL_EXPORTER_OTLP_INSECURE` to the configured
/// endpoint. An insecure exporter is always contacted over plaintext, so an
/// `https://` URL is downgraded to `http://`.
fn resolve_endpoint(endpoint: &str, insecure: bool) -> String {
    match endpoint.strip_prefix("https://") {
        Some(host) if insecure => format!("http://{host}"),
        _ => endpoint.to_string(),
    }
}

/// init_tracing installs the log subscriber and, when `trace` carries an
/// endpoint, an OpenTelemetry layer that exports spans over OTLP/gRPC to that
/// endpoint plus the W3C trace context propagator, which lets incoming
/// `traceparent` headers parent the spans this service records. Spans are
/// attributed to `service_name`. The returned provider must be kept alive for
/// as long as spans are recorded and shut down before exit so buffered spans
/// are flushed. It must be called from within a Tokio runtime.
pub fn init_tracing(
    service_name: &'static str,
    json_logs: bool,
    trace: &TraceConfig,
) -> Result<Option<SdkTracerProvider>> {
    init_tracing_inner(service_name, json_logs, trace, false)
}

/// init_tracing_json installs tracing using the standard structured JSON field names.
pub fn init_tracing_json(
    service_name: &'static str,
    json_logs: bool,
    trace: &TraceConfig,
) -> Result<Option<SdkTracerProvider>> {
    init_tracing_inner(service_name, json_logs, trace, true)
}

fn init_tracing_inner(
    service_name: &'static str,
    json_logs: bool,
    trace: &TraceConfig,
    rest_json: bool,
) -> Result<Option<SdkTracerProvider>> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer: Box<dyn Layer<Registry> + Send + Sync> = if json_logs {
        if rest_json {
            Box::new(fmt::layer().event_format(RestJsonFormat).fmt_fields(JsonFields::new()))
        } else {
            Box::new(
                fmt::layer()
                    .json()
                    .with_file(true)
                    .with_line_number(true)
                    .with_target(true),
            )
        }
    } else {
        Box::new(fmt::layer().with_file(true).with_line_number(true).with_target(true))
    };

    let Some(endpoint) = trace.endpoint.as_deref() else {
        tracing_subscriber::registry().with(fmt_layer).with(env_filter).init();
        return Ok(None);
    };

    let mut builder = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(resolve_endpoint(endpoint, trace.insecure));
    if !trace.insecure
        && let Some(path) = trace.certificate.as_deref()
    {
        let pem = std::fs::read(path).with_context(|| format!("reading OTLP CA certificate {path}"))?;
        builder = builder.with_tls_config(ClientTlsConfig::new().ca_certificate(Certificate::from_pem(pem)));
    }
    let exporter = builder.build()?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(Resource::builder().with_service_name(service_name).build())
        .build();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer(service_name));

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_layer)
        .with(env_filter)
        .init();
    opentelemetry::global::set_tracer_provider(provider.clone());
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    Ok(Some(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace_config(vars: &[(&str, &str)]) -> TraceConfig {
        TraceConfig::from_vars(|key| {
            vars.iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        })
    }

    #[test]
    fn test_trace_config_unset() {
        assert_eq!(trace_config(&[]), TraceConfig::default());
    }

    #[test]
    fn test_trace_config_reads_all_vars() {
        let trace = trace_config(&[
            (ENV_OTLP_ENDPOINT, "https://otel:4317"),
            (ENV_OTLP_INSECURE, "TRUE"),
            (ENV_OTLP_CERTIFICATE, "/etc/otel/ca.crt"),
        ]);
        assert_eq!(trace.endpoint.as_deref(), Some("https://otel:4317"));
        assert!(trace.insecure);
        assert_eq!(trace.certificate.as_deref(), Some("/etc/otel/ca.crt"));
    }

    #[test]
    fn test_trace_config_ignores_blank_values() {
        let trace = trace_config(&[
            (ENV_OTLP_ENDPOINT, "  "),
            (ENV_OTLP_INSECURE, ""),
            (ENV_OTLP_CERTIFICATE, " "),
        ]);
        assert_eq!(trace, TraceConfig::default());
    }

    #[test]
    fn test_trace_config_insecure_requires_true() {
        assert!(!trace_config(&[(ENV_OTLP_INSECURE, "false")]).insecure);
        assert!(!trace_config(&[(ENV_OTLP_INSECURE, "1")]).insecure);
    }

    #[test]
    fn test_resolve_endpoint_downgrades_https_when_insecure() {
        assert_eq!(resolve_endpoint("https://otel:4317", true), "http://otel:4317");
        assert_eq!(resolve_endpoint("https://otel:4317", false), "https://otel:4317");
        assert_eq!(resolve_endpoint("http://otel:4317", true), "http://otel:4317");
    }
}
