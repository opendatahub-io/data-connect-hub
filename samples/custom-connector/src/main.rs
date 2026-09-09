use anyhow::Result;
use clap::Parser;
use flight_service::flight::DataIngestionService;
use flight_service::flight::registry::ConnectorsRegistry;
use flight_service::{CommandLineArgs, configure_metrics, configure_tls, load_config, start_server};
use kube_utils::secrets::KubeSecretStore;
use pg_meta_store::store::PgMetaStore;
use std::sync::Arc;

fn build_connectors_registry() -> ConnectorsRegistry {
    ConnectorsRegistry::new()
    // .with_connector(Arc::new(MyCustomConnector::new()))
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

    tracing::info!("Starting custom connector Flight service");

    let addr: std::net::SocketAddr = format!("{}:{}", config.server.address, config.server.port).parse()?;
    let builder = tonic::transport::Server::builder();
    let builder = configure_tls(builder, &config.tls).await?;
    configure_metrics(&config)?;

    let connectors_registry = Arc::new(build_connectors_registry());
    let secret_store = Arc::new(KubeSecretStore::try_default().await?);
    let query_options = commons::api::connector::QueryOptions {
        batch_size: config.query.batch_size,
    };

    let tenant_id = config.global_connection_types.tenant_id;
    let auth = config.auth;
    let meta_store = Arc::new(PgMetaStore::new(config.database, tenant_id).await?);

    let service = DataIngestionService::new(connectors_registry, meta_store, secret_store, query_options);

    start_server(builder, &auth, service, addr).await?;
    tracing::info!("Custom connector Flight service stopped");
    Ok(())
}
