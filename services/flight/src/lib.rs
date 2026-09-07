pub mod flight;
pub mod utils;

use anyhow::Result;
use arrow_flight::flight_service_server::FlightServiceServer;
use clap::Parser;
use config::{Config, File};
use flight::DataIngestionService;
use flight::auth::AuthLayer;
use flight::metrics::{install_prometheus_recorder, spawn_metrics_server};
use kube_utils::KubeAuthClient;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use utils::ServerConfig;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct CommandLineArgs {
    /// Enable JSON logs
    #[arg(short, long, default_value = "false")]
    pub json_logs: bool,

    /// Config file for this server
    #[arg(short, long, default_value = "config/config.toml")]
    pub config: String,

    /// Optional additional config file (e.g. a mounted Secret) merged on top
    /// of `config`; missing values here fall back to `config`.
    #[arg(long, default_value = "/secrets/secret-config.toml")]
    pub secret_config: String,
}

pub fn load_config(config_file: String, secret_config_file: String) -> Result<ServerConfig> {
    let config = Config::builder()
        .add_source(File::with_name(config_file.as_str()))
        .add_source(File::with_name(secret_config_file.as_str()).required(false))
        .build()?;

    let config: ServerConfig = config.try_deserialize()?;
    Ok(config)
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => println!("\nReceived Ctrl+C, shutting down gracefully..."),
        _ = terminate => println!("\nReceived SIGTERM, shutting down gracefully..."),
    }
}

pub async fn configure_tls(
    mut builder: tonic::transport::Server,
    tls: &utils::TlsConfig,
) -> Result<tonic::transport::Server> {
    tls.validate().map_err(|e| anyhow::anyhow!(e))?;
    if let (Some(cert_file), Some(key_file)) = (&tls.cert_file, &tls.key_file) {
        let cert = tokio::fs::read(cert_file).await?;
        let key = tokio::fs::read(key_file).await?;
        let identity = tonic::transport::Identity::from_pem(cert, key);
        let tls_config = tonic::transport::ServerTlsConfig::new().identity(identity);
        builder = builder.tls_config(tls_config)?;
        tracing::info!("TLS enabled (cert: {}, key: {})", cert_file, key_file);
    } else {
        tracing::warn!("TLS is DISABLED — gRPC traffic is unencrypted");
    }
    Ok(builder)
}

pub fn configure_metrics(config: &ServerConfig) -> Result<()> {
    if config.metrics.enabled {
        tracing::info!(
            "Prometheus metrics enabled on {}:{}",
            config.metrics.address,
            config.metrics.port
        );
        install_prometheus_recorder()?;
        spawn_metrics_server(config.metrics.address.clone(), config.metrics.port);
    } else {
        tracing::info!("Prometheus metrics disabled");
    }
    Ok(())
}

pub async fn start_server(
    mut builder: tonic::transport::Server,
    auth: &utils::AuthConfig,
    data_service: DataIngestionService,
    addr: std::net::SocketAddr,
) -> Result<()> {
    let service = FlightServiceServer::new(data_service);
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<FlightServiceServer<DataIngestionService>>()
        .await;

    if auth.enabled {
        tracing::info!(
            "Auth enabled (cache TTL: {}s, token_review_audiences: {:?})",
            auth.cache_ttl_secs,
            auth.token_review_audiences
        );
        let kube_auth = KubeAuthClient::try_default(
            Duration::from_secs(auth.cache_ttl_secs),
            auth.token_review_audiences.clone(),
        )
        .await?;
        let auth_layer = AuthLayer::new(Arc::new(kube_auth));
        builder
            .layer(auth_layer)
            .add_service(health_service)
            .add_service(service)
            .serve_with_shutdown(addr, shutdown_signal())
            .await?;
    } else {
        tracing::warn!("Auth is DISABLED — all requests are unauthenticated");
        builder
            .add_service(health_service)
            .add_service(service)
            .serve_with_shutdown(addr, shutdown_signal())
            .await?;
    }

    Ok(())
}
