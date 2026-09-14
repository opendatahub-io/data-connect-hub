use anyhow::Result;
use clap::Parser;
#[cfg(feature = "elasticsearch")]
use elasticsearch_connector::ElasticsearchConnector;
use flight_service::flight::DataIngestionService;
use flight_service::flight::registry::ConnectorsRegistry;
use flight_service::utils::ServerConfig;
use flight_service::{CommandLineArgs, configure_metrics, configure_tls, load_config, start_server};
use kube_utils::secrets::KubeSecretStore;
#[cfg(feature = "milvus")]
use milvus_connector::MilvusConnector;
#[cfg(feature = "neo4j")]
use neo4j_connector::Neo4jConnector;
use pg_meta_store::store::PgMetaStore;
#[cfg(feature = "postgres")]
use postgres_connector::PgConnector;
#[cfg(feature = "s3")]
use s3_connector::S3Connector;
#[cfg(feature = "sqlite")]
use sqlite_connector::SqliteConnector;
use std::sync::Arc;
use std::time::Duration;
#[cfg(feature = "uri")]
use uri_connector::UriConnector;

#[allow(unused_variables)]
fn build_connectors_registry(config: &ServerConfig) -> ConnectorsRegistry {
    let cache = &config.ingestion_cache_pools;
    let connectors = &config.connectors;
    let cache_ttl = Duration::from_secs(cache.ttl_secs);
    let cache_idle = Duration::from_secs(cache.idle_secs);
    let cache_cap = cache.max_capacity;

    #[allow(unused_mut)]
    let mut registry = ConnectorsRegistry::new();

    #[cfg(feature = "postgres")]
    {
        let pg = connectors.postgres();
        if pg.enabled {
            registry = registry.with_connector(Arc::new(PgConnector::new(cache_ttl, cache_idle, cache_cap, pg)));
        }
    }

    #[cfg(feature = "sqlite")]
    {
        let sqlite = connectors.sqlite();
        if sqlite.enabled {
            registry = registry.with_connector(Arc::new(SqliteConnector::new(sqlite)));
        }
    }

    #[cfg(feature = "s3")]
    {
        let s3 = connectors.s3();
        if s3.enabled {
            registry = registry.with_connector(Arc::new(S3Connector::new(cache_ttl, cache_idle, cache_cap, s3)));
        }
    }

    #[cfg(feature = "milvus")]
    {
        let milvus = connectors.milvus();
        if milvus.enabled {
            registry =
                registry.with_connector(Arc::new(MilvusConnector::new(cache_ttl, cache_idle, cache_cap, milvus)));
        }
    }

    #[cfg(feature = "elasticsearch")]
    {
        let es = connectors.elasticsearch();
        if es.enabled {
            registry = registry.with_connector(Arc::new(ElasticsearchConnector::new(
                cache_ttl, cache_idle, cache_cap, es,
            )));
        }
    }

    #[cfg(feature = "neo4j")]
    {
        let neo4j = connectors.neo4j();
        if neo4j.enabled {
            registry = registry.with_connector(Arc::new(Neo4jConnector::new(cache_ttl, cache_idle, cache_cap, neo4j)));
        }
    }

    #[cfg(feature = "uri")]
    {
        let uri = connectors.uri();
        if uri.enabled {
            registry = registry.with_connector(Arc::new(UriConnector::new(cache_ttl, cache_idle, cache_cap, uri)));
        }
    }

    registry
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
    let secret_store = Arc::new(KubeSecretStore::try_default().await?);
    let query_options = commons::api::connector::QueryOptions {
        batch_size: config.query.batch_size,
    };

    let tenant_id = config.global_connection_types.tenant_id;
    let auth = config.auth;
    let meta_store = Arc::new(PgMetaStore::new(config.database, tenant_id).await?);

    let service = DataIngestionService::new(connectors_registry, meta_store, secret_store, query_options);

    start_server(builder, &auth, service, addr).await?;
    tracing::info!("DataConnectorHub Flight service stopped");
    Ok(())
}
