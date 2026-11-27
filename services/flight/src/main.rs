use anyhow::Result;
use clap::Parser;
use commons::utils::audit::AuditConfig;
use commons::utils::{TraceConfig, init_tracing_json, log_trace_exporter};
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

/// SERVICE_NAME identifies this service in exported traces.
const SERVICE_NAME: &str = "dch-flight-service";

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

mod fips;

/// Key exchange groups are restricted to the NIST curves first. `rustls`
/// computes `CryptoProvider::fips()` as an `all()` across every configured
/// component, and derives `ClientConfig::require_ems` from that value, so a
/// single component reporting non-FIPS would silently disable Extended Master
/// Secret — mandatory for the TLS 1.2 KDF under FIPS 140-3 IG section D.Q.
///
/// When `DCH_REQUIRE_FIPS` is set, a configuration that does not report FIPS
/// is treated as fatal rather than degrading quietly.
///
/// The status is returned rather than logged: this has to run before
/// `init_tracing_json`, which may open an OTLP exporter over TLS, so no
/// subscriber exists yet and anything logged here would be discarded.
struct CryptoStatus {
    context_fips: bool,
    provider_fips: bool,
    cipher_suites_fips: bool,
    kx_groups_fips: bool,
    signature_verification_fips: bool,
    secure_random_fips: bool,
    key_provider_fips: bool,
    config_fips: bool,
    kx_groups: Vec<rustls::NamedGroup>,
    cipher_suites: Vec<rustls::CipherSuite>,
    signature_schemes: Vec<rustls::SignatureScheme>,
}

impl CryptoStatus {
    fn log(&self) {
        tracing::info!(
            context_fips = self.context_fips,
            provider_fips = self.provider_fips,
            cipher_suites_fips = self.cipher_suites_fips,
            kx_groups_fips = self.kx_groups_fips,
            signature_verification_fips = self.signature_verification_fips,
            secure_random_fips = self.secure_random_fips,
            key_provider_fips = self.key_provider_fips,
            config_fips = self.config_fips,
            kx_groups = ?self.kx_groups,
            cipher_suites = ?self.cipher_suites,
            signature_schemes = ?self.signature_schemes,
            "installed OpenSSL-backed rustls crypto provider"
        );
    }
}

fn install_crypto_provider() -> Result<CryptoStatus> {
    use rustls::crypto::CryptoProvider;

    fips::load_default_openssl_config()?;

    let mut provider = rustls_ossl::try_default_provider()
        .map_err(|e| anyhow::anyhow!("the OpenSSL crypto provider is unusable: {e}"))?;

    fips::retain_fips_kx_groups(&mut provider);

    let kx_groups: Vec<_> = provider.kx_groups.iter().map(|g| g.name()).collect();
    // Capture the resolved algorithm lists before reading the flags. Every
    // FIPS flag here is an `all()`, which is true over an empty list, so a
    // provider that resolved no cipher suites, no key exchange groups or no
    // signature algorithms reports flawless compliance while being unable to
    // complete a handshake — an empty signature_algorithms extension does not
    // even encode to a valid ClientHello. Treat an empty list as fatal rather
    // than letting it surface later as a TLS error with no stated cause.
    let cipher_suites: Vec<_> = provider.cipher_suites.iter().map(|suite| suite.suite()).collect();
    let signature_schemes: Vec<_> = provider
        .signature_verification_algorithms
        .mapping
        .iter()
        .map(|(scheme, _)| *scheme)
        .collect();

    if cipher_suites.is_empty() || kx_groups.is_empty() || signature_schemes.is_empty() {
        return Err(anyhow::anyhow!(
            "the OpenSSL provider resolved an incomplete algorithm set \
         (cipher suites: {}, kx groups: {}, signature algorithms: {}); refusing to start",
            cipher_suites.len(),
            kx_groups.len(),
            signature_schemes.len(),
        ));
    }

    let cipher_suites_fips = provider.cipher_suites.iter().all(|suite| suite.fips());
    let kx_groups_fips = provider.kx_groups.iter().all(|group| group.fips());
    let signature_verification_fips = provider.signature_verification_algorithms.fips();
    let secure_random_fips = provider.secure_random.fips();
    let key_provider_fips = provider.key_provider.fips();
    let provider_fips = provider.fips();
    let context_fips = rustls_ossl::osslctx().fips_is_enabled();

    CryptoProvider::install_default(provider)
        .map_err(|_| anyhow::anyhow!("a rustls CryptoProvider was already installed"))?;

    // Build a throwaway client configuration purely to read the config-level
    // FIPS status: it folds in `require_ems`, which the provider-level flag
    // does not cover. tonic builds its own configurations internally, so
    // there is no live one to inspect here.
    let probe = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let config_fips = probe.fips();

    if std::env::var_os("DCH_REQUIRE_FIPS").is_some() && !config_fips {
        return Err(anyhow::anyhow!(
            "DCH_REQUIRE_FIPS is set but the TLS configuration does not report FIPS \
         (provider_fips={provider_fips}, config_fips={config_fips}); refusing to start"
        ));
    }

    Ok(CryptoStatus {
        context_fips,
        provider_fips,
        cipher_suites_fips,
        kx_groups_fips,
        signature_verification_fips,
        secure_random_fips,
        key_provider_fips,
        config_fips,
        kx_groups,
        cipher_suites,
        signature_schemes,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let crypto_status = install_crypto_provider()?;
    let args = CommandLineArgs::parse();
    let config = load_config(args.config, args.secret_config)?;
    config.query.validate().map_err(|e| anyhow::anyhow!(e))?;

    let trace = TraceConfig::from_env();
    let audit = AuditConfig::from_env();
    let tracer_provider = init_tracing_json(SERVICE_NAME, args.json_logs, &trace)?;

    crypto_status.log();

    tracing::info!("Starting DataConnectorHub Flight service");
    tracing::info!(
        audit_enabled = audit.enabled,
        audit_log_queries = audit.log_queries,
        "Audit logging is {}, query logging is {}",
        if audit.enabled { "enabled" } else { "disabled" },
        if audit.log_queries { "enabled" } else { "disabled" }
    );
    log_trace_exporter(&trace);

    let addr: std::net::SocketAddr = format!("{}:{}", config.server.address, config.server.port).parse()?;
    let builder = tonic::transport::Server::builder();
    let (builder, tls_acceptor) = configure_tls(builder, &config.tls).await?;
    configure_metrics(&config)?;

    let connectors_registry = Arc::new(build_connectors_registry(&config));
    let secret_store = Arc::new(KubeSecretStore::try_default().await?);
    let query_options = commons::api::connector::QueryOptions {
        batch_size: config.query.batch_size,
    };

    let global_tenant_ids = config.global_connection_types.tenant_ids;
    let auth = config.auth;
    let meta_store = Arc::new(PgMetaStore::new(config.database, global_tenant_ids).await?);

    let service =
        DataIngestionService::new(connectors_registry, meta_store, secret_store, query_options).with_audit(audit);

    let server_result = start_server(builder, tls_acceptor, &auth, service, addr).await;
    tracing::info!("DataConnectorHub Flight service stopped");

    if let Some(provider) = tracer_provider
        && let Err(e) = provider.shutdown()
    {
        tracing::warn!(error = %e, "Failed to flush traces on shutdown");
    }

    server_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use commons::utils::config::GlobalConnectionTypes;
    use flight_service::utils::{
        AuthConfig, ConnectorsConfig, IngestionCachePools, MetricsConfig, QueryConfig, Server, TlsConfig,
    };
    use pg_meta_store::store::DatabaseConfig;
    use std::collections::HashSet;

    fn server_config(connectors: ConnectorsConfig) -> ServerConfig {
        ServerConfig {
            server: Server {
                address: "127.0.0.1".to_string(),
                port: 0,
            },
            database: DatabaseConfig {
                url: "postgresql://localhost/test".to_string(),
            },
            ingestion_cache_pools: IngestionCachePools {
                max_capacity: 1,
                ttl_secs: 1,
                idle_secs: 1,
            },
            connectors,
            query: QueryConfig::default(),
            auth: AuthConfig::default(),
            metrics: MetricsConfig::default(),
            tls: TlsConfig::default(),
            global_connection_types: GlobalConnectionTypes::new(vec!["test".to_string()]),
        }
    }

    fn registry_names(registry: &ConnectorsRegistry) -> HashSet<String> {
        registry
            .get_supported_connectors()
            .into_iter()
            .map(|connector| connector.provider())
            .collect()
    }

    #[test]
    fn registry_with_default_disabled() {
        // With the default disabled, only explicitly enabled PostgreSQL is registered.
        let config_toml = r#"
[default]
enabled = false

[postgres]
enabled = true

[s3]
enabled = false
"#;
        let config = config::Config::builder()
            .add_source(config::File::from_str(config_toml, config::FileFormat::Toml))
            .build()
            .unwrap();
        let connectors: ConnectorsConfig = config.try_deserialize().unwrap();
        let registry = build_connectors_registry(&server_config(connectors));

        assert_eq!(registry_names(&registry), HashSet::from(["postgres".to_string()]));
    }

    #[test]
    fn registry_with_default_enabled() {
        // With the default enabled, unspecified connectors are registered and explicitly disabled S3 is omitted.
        let config_toml = r#"
[default]
enabled = true

[postgres]
enabled = true

[s3]
enabled = false
"#;
        let config = config::Config::builder()
            .add_source(config::File::from_str(config_toml, config::FileFormat::Toml))
            .build()
            .unwrap();
        let connectors: ConnectorsConfig = config.try_deserialize().unwrap();
        let registry = build_connectors_registry(&server_config(connectors));

        assert_eq!(
            registry_names(&registry),
            HashSet::from([
                "postgres".to_string(),
                "sqlite".to_string(),
                "milvus".to_string(),
                "elasticsearch".to_string(),
                "neo4j".to_string(),
                "uri".to_string(),
            ])
        );
    }
}
