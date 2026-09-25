use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{App, Error, HttpResponse, HttpServer, http::header, web};
use metrics::{Unit, counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use std::net::{SocketAddr, TcpListener};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tracing::{error, info};

const REST_REQUESTS_TOTAL: &str = "dch_rest_requests_total";
const REST_REQUEST_DURATION_SECONDS: &str = "dch_rest_request_duration_seconds";
const REST_REQUESTS_ACTIVE: &str = "dch_rest_requests_active";
const UNMATCHED_ROUTE: &str = "unmatched";
const REQUEST_DURATION_BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
static METRICS_DESCRIBED: OnceLock<()> = OnceLock::new();

pub fn install_prometheus_recorder() -> anyhow::Result<()> {
    if PROMETHEUS_HANDLE.get().is_some() {
        return Ok(());
    }

    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(REST_REQUEST_DURATION_SECONDS.to_owned()),
            REQUEST_DURATION_BUCKETS,
        )
        .map_err(|error| anyhow::anyhow!("invalid REST request duration buckets: {error}"))?
        .install_recorder()
        .map_err(|e| anyhow::anyhow!("failed to install Prometheus recorder: {e}"))?;

    let _ = PROMETHEUS_HANDLE.set(handle);
    METRICS_DESCRIBED.get_or_init(|| {
        describe_counter!(
            REST_REQUESTS_TOTAL,
            Unit::Count,
            "Total number of REST requests by method/route/status"
        );
        describe_histogram!(
            REST_REQUEST_DURATION_SECONDS,
            Unit::Seconds,
            "REST request handler duration in seconds by method/route/status. For streaming responses, this measures setup time before body consumption."
        );
        describe_gauge!(
            REST_REQUESTS_ACTIVE,
            Unit::Count,
            "Number of REST request handlers currently executing by method/route"
        );
    });
    Ok(())
}

fn method_label(method: &actix_web::http::Method) -> &'static str {
    match method.as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "CONNECT" => "CONNECT",
        "TRACE" => "TRACE",
        _ => "OTHER",
    }
}

fn observe_request(method: &'static str, route: &str, status: &str, duration: Duration) {
    if PROMETHEUS_HANDLE.get().is_none() {
        return;
    }

    counter!(
        REST_REQUESTS_TOTAL,
        "method" => method,
        "route" => route.to_owned(),
        "status" => status.to_owned()
    )
    .increment(1);

    histogram!(
        REST_REQUEST_DURATION_SECONDS,
        "method" => method,
        "route" => route.to_owned(),
        "status" => status.to_owned()
    )
    .record(duration.as_secs_f64());
}

struct InFlightGuard {
    method: &'static str,
    route: String,
}

impl InFlightGuard {
    fn new(method: &'static str, route: String) -> Self {
        if PROMETHEUS_HANDLE.get().is_some() {
            gauge!(REST_REQUESTS_ACTIVE, "method" => method, "route" => route.clone()).increment(1.0);
        }
        Self { method, route }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if PROMETHEUS_HANDLE.get().is_some() {
            gauge!(REST_REQUESTS_ACTIVE, "method" => self.method, "route" => self.route.clone()).decrement(1.0);
        }
    }
}

pub async fn observe_http_request(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    let started = Instant::now();
    let method = method_label(req.method());
    let route = req.match_pattern().unwrap_or_else(|| UNMATCHED_ROUTE.to_owned());
    let _in_flight = InFlightGuard::new(method, route.clone());

    let response = next.call(req).await;
    let status = match &response {
        Ok(response) => response.status(),
        Err(error) => error.as_response_error().status_code(),
    };
    observe_request(method, &route, status.as_str(), started.elapsed());
    response
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

pub fn spawn_metrics_server(address: String, port: u16) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{address}:{port}")
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid metrics listen address '{address}:{port}': {error}"))?;
    let listener = TcpListener::bind(addr)
        .map_err(|error| anyhow::anyhow!("failed to bind metrics endpoint on {addr}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| anyhow::anyhow!("failed to configure metrics endpoint on {addr}: {error}"))?;

    std::thread::spawn(move || {
        actix_web::rt::System::new().block_on(async move {
            let server = match HttpServer::new(|| App::new().route("/metrics", web::get().to(metrics_handler)))
                .listen(listener)
            {
                Ok(server) => server.run(),
                Err(error) => {
                    error!("failed to start metrics endpoint on {}: {}", addr, error);
                    return;
                },
            };

            info!("Prometheus metrics endpoint listening on http://{}/metrics", addr);
            if let Err(error) = server.await {
                error!("metrics server terminated with error: {}", error);
            }
        });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{http::StatusCode, middleware, test as actix_test};

    fn metric_line<'a>(body: &'a str, name: &str, labels: &[(&str, &str)]) -> Option<&'a str> {
        body.lines().find(|line| {
            line.starts_with(name)
                && labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!(r#"{key}="{value}""#)))
        })
    }

    #[actix_web::test]
    async fn records_normalized_routes_statuses_and_active_requests() {
        install_prometheus_recorder().unwrap();
        let app = actix_test::init_service(
            App::new()
                .wrap(middleware::from_fn(observe_http_request))
                .route("/connections/{id}", web::get().to(HttpResponse::NoContent))
                .service(
                    web::scope("/api")
                        .wrap(middleware::from_fn(crate::rest::middleware::validate_headers))
                        .route("/connections", web::get().to(HttpResponse::Ok)),
                ),
        )
        .await;

        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/connections/private-id")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/not-found/private-id").to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/api/connections").to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = render_prometheus().unwrap();
        assert!(
            metric_line(
                &body,
                REST_REQUESTS_TOTAL,
                &[("method", "GET"), ("route", "/connections/{id}"), ("status", "204")]
            )
            .is_some()
        );
        assert!(
            metric_line(
                &body,
                REST_REQUEST_DURATION_SECONDS,
                &[("method", "GET"), ("route", "/connections/{id}"), ("status", "204")]
            )
            .is_some()
        );
        assert!(
            metric_line(
                &body,
                REST_REQUESTS_TOTAL,
                &[("method", "GET"), ("route", UNMATCHED_ROUTE), ("status", "404")]
            )
            .is_some()
        );
        assert!(
            metric_line(
                &body,
                REST_REQUESTS_TOTAL,
                &[("method", "GET"), ("route", "/api/connections"), ("status", "400")]
            )
            .is_some()
        );
        assert!(body.contains(&format!("# TYPE {REST_REQUEST_DURATION_SECONDS} histogram")));
        assert!(body.contains(&format!("{REST_REQUEST_DURATION_SECONDS}_bucket{{")));
        assert!(body.contains("# TYPE dch_rest_requests_active gauge"));
        assert!(!body.contains("private-id"));

        let response = metrics_handler().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/plain; version=0.0.4; charset=utf-8"
        );

        let in_flight = InFlightGuard::new("GET", "/active-test".to_owned());
        let body = render_prometheus().unwrap();
        let line = metric_line(
            &body,
            REST_REQUESTS_ACTIVE,
            &[("method", "GET"), ("route", "/active-test")],
        )
        .unwrap();
        assert!(line.ends_with(" 1"));
        drop(in_flight);

        let body = render_prometheus().unwrap();
        let line = metric_line(
            &body,
            REST_REQUESTS_ACTIVE,
            &[("method", "GET"), ("route", "/active-test")],
        )
        .unwrap();
        assert!(line.ends_with(" 0"));
    }

    #[test]
    fn bounds_unknown_http_methods() {
        let method = actix_web::http::Method::from_bytes(b"CUSTOM").unwrap();
        assert_eq!(method_label(&method), "OTHER");
    }
}
