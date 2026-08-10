use crate::utils::ServerConfig;

use anyhow::Result;
use arrow_flight::flight_service_server::FlightServiceServer;
use clap::Parser;
use config::{Config, File};
use flight_service::flight::TabularDataService;
use flight_service::flight::auth::AuthInterceptor;
use flight_service::flight::registry::ConnectorsRegistry;
use kube_utils::secrets::KubeSecretStore;
use pg_meta_store::store::PgMetaStore;
use postgres_connector::PgConnector;
use sqlite_connector::SqliteConnector;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;

mod utils;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CommandLineArgs {
    /// Enable JSON logs
    #[arg(short, long, default_value = "false")]
    json_logs: bool,

    /// Config file for this server
    #[arg(short, long, default_value = "config/config.toml")]
    config: String,

    /// Optional additional config file (e.g. a mounted Secret) merged on top
    /// of `config`; missing values here fall back to `config`.
    #[arg(long, default_value = "/secrets/secret-config.toml")]
    secret_config: String,
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

    // Wait until either Ctrl+C (SIGINT) or SIGTERM is received
    tokio::select! {
        _ = ctrl_c => println!("\nReceived Ctrl+C, shutting down gracefully..."),
        _ = terminate => println!("\nReceived SIGTERM, shutting down gracefully..."),
    }
}

fn load_config(config_file: String, secret_config_file: String) -> Result<ServerConfig> {
    let config = Config::builder()
        .add_source(File::with_name(config_file.as_str()))
        .add_source(File::with_name(secret_config_file.as_str()).required(false))
        .build()?;

    let config: ServerConfig = config.try_deserialize()?;
    Ok(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CommandLineArgs::parse();
    let config = load_config(args.config, args.secret_config)?;
    config.query.validate().map_err(|e| anyhow::anyhow!(e))?;
    commons::utils::init_tracing(args.json_logs);

    tracing::info!("Starting DataConnectorHub Flight service");

    let connectors_registry = ConnectorsRegistry::new()
        .with_connector(Arc::new(PgConnector::new(
            Duration::from_secs(config.ingestion_cache_pools.ttl_secs),
            Duration::from_secs(config.ingestion_cache_pools.idle_secs),
            config.ingestion_cache_pools.max_capacity,
        )))
        .with_connector(Arc::new(SqliteConnector::new()));

    let secret_store = KubeSecretStore::try_default(Duration::from_secs(300)).await?;

    let query_options = commons::api::tabular::QueryOptions {
        batch_size: config.query.batch_size,
    };

    let addr = format!("{}:{}", config.server.address, config.server.port).parse()?;
    let service = TabularDataService::new(
        Arc::new(connectors_registry),
        Arc::new(PgMetaStore::new(config.database).await?),
        Arc::new(secret_store),
        query_options,
    );

    let auth_interceptor = AuthInterceptor::new();
    let service = FlightServiceServer::with_interceptor(service, auth_interceptor);

    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<FlightServiceServer<TabularDataService>>()
        .await;

    tonic::transport::Server::builder()
        .add_service(health_service)
        .add_service(service)
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    tracing::info!("DataConnectorHub Flight service stopped");

    Ok(())
}
