use actix_web::{App, HttpResponse, HttpServer, http::header, web};
use metrics::{Unit, counter, describe_counter, describe_histogram, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{error, info};

const FLIGHT_RPC_REQUESTS_TOTAL: &str = "dch_flight_rpc_requests_total";
const FLIGHT_RPC_DURATION_SECONDS: &str = "dch_flight_rpc_duration_seconds";

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
static METRICS_DESCRIBED: OnceLock<()> = OnceLock::new();

pub fn install_prometheus_recorder() -> anyhow::Result<()> {
    if PROMETHEUS_HANDLE.get().is_some() {
        return Ok(());
    }

    let handle = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| anyhow::anyhow!("failed to install Prometheus recorder: {e}"))?;

    // Best-effort set; if another thread won the race we can continue.
    let _ = PROMETHEUS_HANDLE.set(handle);
    METRICS_DESCRIBED.get_or_init(|| {
        describe_counter!(
            FLIGHT_RPC_REQUESTS_TOTAL,
            Unit::Count,
            "Total number of Flight RPC requests by method/operation/status"
        );
        describe_histogram!(
            FLIGHT_RPC_DURATION_SECONDS,
            Unit::Seconds,
            "Flight RPC handler duration in seconds by method/operation/status. For streaming RPCs, this measures setup time before stream consumption."
        );
    });
    Ok(())
}

pub fn observe_rpc(method: &'static str, operation: &'static str, status: &'static str, duration: Duration) {
    // Metrics are disabled when the recorder is not installed.
    if PROMETHEUS_HANDLE.get().is_none() {
        return;
    }

    counter!(
        FLIGHT_RPC_REQUESTS_TOTAL,
        "method" => method,
        "operation" => operation,
        "status" => status
    )
    .increment(1);

    histogram!(
        FLIGHT_RPC_DURATION_SECONDS,
        "method" => method,
        "operation" => operation,
        "status" => status
    )
    .record(duration.as_secs_f64());
}

fn render_prometheus() -> Option<String> {
    PROMETHEUS_HANDLE.get().map(PrometheusHandle::render)
}

async fn metrics_handler() -> HttpResponse {
    match render_prometheus() {
        Some(body) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8"))
            .body(body),
        None => HttpResponse::ServiceUnavailable().body("metrics recorder not installed\n"),
    }
}

pub fn spawn_metrics_server(address: String, port: u16) {
    std::thread::spawn(move || {
        let addr: SocketAddr = match format!("{address}:{port}").parse() {
            Ok(a) => a,
            Err(e) => {
                error!("invalid metrics listen address '{}:{}': {}", address, port, e);
                return;
            },
        };

        actix_web::rt::System::new().block_on(async move {
            let server =
                match HttpServer::new(|| App::new().route("/metrics", web::get().to(metrics_handler))).bind(addr) {
                    Ok(s) => s.run(),
                    Err(e) => {
                        error!("failed to bind metrics endpoint on {}: {}", addr, e);
                        return;
                    },
                };

            info!("Prometheus metrics endpoint listening on http://{}/metrics", addr);
            if let Err(e) = server.await {
                error!("metrics server terminated with error: {}", e);
            }
        });
    });
}
