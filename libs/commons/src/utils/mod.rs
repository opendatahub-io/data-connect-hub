pub mod config;

use anyhow::{Context, Result};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tonic::transport::{Certificate, ClientTlsConfig};
use tracing::info;
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

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
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer: Box<dyn Layer<Registry> + Send + Sync> = if json_logs {
        Box::new(
            fmt::layer()
                .json()
                .with_file(true)
                .with_line_number(true)
                .with_target(true),
        )
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
