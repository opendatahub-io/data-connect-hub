use crate::utils::ServerConfig;

use anyhow::Result;
use arrow_flight::flight_service_server::FlightServiceServer;
use clap::Parser;
use config::{Config, File};
use elasticsearch_connector::ElasticsearchConnector;
use flight_service::flight::DataIngestionService;
use flight_service::flight::auth::AuthLayer;
use flight_service::flight::metrics::{install_prometheus_recorder, spawn_metrics_server};
use flight_service::flight::registry::ConnectorsRegistry;
use kube_utils::KubeAuthClient;
use kube_utils::secrets::KubeSecretStore;
use milvus_connector::MilvusConnector;
use neo4j_connector::Neo4jConnector;
use pg_meta_store::store::PgMetaStore;
use postgres_connector::PgConnector;
use s3_connector::S3Connector;
use sqlite_connector::SqliteConnector;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use uri_connector::UriConnector;

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

fn build_connectors_registry(config: &ServerConfig) -> ConnectorsRegistry {
    let cache = &config.ingestion_cache_pools;
    let connectors = &config.connectors;
    let cache_ttl = Duration::from_secs(cache.ttl_secs);
    let cache_idle = Duration::from_secs(cache.idle_secs);
    let cache_cap = cache.max_capacity;

    let mut registry = ConnectorsRegistry::new();

    let pg = connectors.postgres();
    if pg.enabled {
        registry = registry.with_connector(Arc::new(PgConnector::new(cache_ttl, cache_idle, cache_cap, pg)));
    }

    let sqlite = connectors.sqlite();
    if sqlite.enabled {
        registry = registry.with_connector(Arc::new(SqliteConnector::new(sqlite)));
    }

    let s3 = connectors.s3();
    if s3.enabled {
        registry = registry.with_connector(Arc::new(S3Connector::new(cache_ttl, cache_idle, cache_cap, s3)));
    }

    let milvus = connectors.milvus();
    if milvus.enabled {
        registry = registry.with_connector(Arc::new(MilvusConnector::new(cache_ttl, cache_idle, cache_cap, milvus)));
    }

    let es = connectors.elasticsearch();
    if es.enabled {
        registry = registry.with_connector(Arc::new(ElasticsearchConnector::new(
            cache_ttl, cache_idle, cache_cap, es,
        )));
    }

    let neo4j = connectors.neo4j();
    if neo4j.enabled {
        registry = registry.with_connector(Arc::new(Neo4jConnector::new(cache_ttl, cache_idle, cache_cap, neo4j)));
    }

    let uri = connectors.uri();
    if uri.enabled {
        registry = registry.with_connector(Arc::new(UriConnector::new(cache_ttl, cache_idle, cache_cap, uri)));
    }

    registry
}

async fn configure_tls(
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

fn configure_metrics(config: &ServerConfig) -> Result<()> {
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

async fn start_server(
    mut builder: tonic::transport::Server,
    auth: &utils::AuthConfig,
    service: FlightServiceServer<DataIngestionService>,
    addr: std::net::SocketAddr,
) -> Result<()> {
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

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls CryptoProvider");

    let args = CommandLineArgs::parse();
    let config = load_config(args.config, args.secret_config)?;
    config.query.validate().map_err(|e| anyhow::anyhow!(e))?;
    commons::utils::init_tracing(args.json_logs);

    tracing::info!("Starting DataConnectorHub Flight service");

    let addr: std::net::SocketAddr = format!("{}:{}", config.server.address, config.server.port).parse()?;
    let builder = tonic::transport::Server::builder();
    let builder = configure_tls(builder, &config.tls).await?;
    configure_metrics(&config)?;

    let connectors_registry = Arc::new(build_connectors_registry(&config));
    let secret_store = Arc::new(KubeSecretStore::try_default(Duration::from_secs(300)).await?);
    let query_options = commons::api::connector::QueryOptions {
        batch_size: config.query.batch_size,
    };

    let tenant_id = config.global_connection_types.tenant_id;
    let auth = config.auth;
    let meta_store = Arc::new(PgMetaStore::new(config.database, tenant_id).await?);

    let service = FlightServiceServer::new(DataIngestionService::new(
        connectors_registry,
        meta_store,
        secret_store,
        query_options,
    ));

    start_server(builder, &auth, service, addr).await?;
    tracing::info!("DataConnectorHub Flight service stopped");
    Ok(())
}
