use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use clap::Parser;

use crate::rest::API_VERSION;
use crate::rest::endpoints::*;
use crate::rest::errors::{json_config, path_config, query_config};
use crate::rest::metrics::{install_prometheus_recorder, observe_http_request, spawn_metrics_server};
use crate::rest::middleware::{trace_request, validate_headers};
use crate::state::ApiService;
use crate::utils::ServerConfig;
use anyhow::Result;
use commons::api::storage::MetaStore;
use commons::utils::audit::AuditConfig;
use commons::utils::{TraceConfig, init_tracing_json, log_trace_exporter};
use config::{Config, File};
use kube_utils::secrets::KubeSecretStore;
use pg_meta_store::store::PgMetaStore;
use std::sync::Arc;
use url::Url;

/// SERVICE_NAME identifies this service in exported traces.
const SERVICE_NAME: &str = "dch-rest-service";

mod auth;
mod clients;
mod rest;
#[allow(unused)]
mod state;
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

fn api_routes(cfg: &mut web::ServiceConfig, _service: Arc<ApiService>) {
    cfg.route("/health", web::get().to(health))
        .service(
            web::scope(&format!("/api/{API_VERSION}/data"))
                .wrap(middleware::from_fn(validate_headers))
                .wrap(middleware::from_fn(trace_request))
                .route("/connection-types", web::get().to(list_connection_types))
                .route("/connection-types", web::post().to(create_connection_type))
                .route("/connection-types/{id}", web::get().to(get_connection_type))
                .route("/connection-types/{id}", web::patch().to(patch_connection_type))
                .route("/connection-types/{id}", web::delete().to(delete_connection_type))
                .route("/connections", web::get().to(list_connections))
                .route("/connections", web::post().to(create_connection))
                .route("/connections/{id}", web::get().to(get_connection))
                .route("/connections/{id}", web::patch().to(patch_connection))
                .route("/connections/{id}", web::delete().to(delete_connection))
                .route(
                    "/connections/{id}/exports/secrets/{secret_name}",
                    web::put().to(export_connection),
                )
                .route("/connections/{id}/readiness", web::post().to(check_existent_connection))
                .route("/connections/{id}/binary", web::get().to(get_binary_data))
                .route("/test/credentials", web::post().to(test_credentials))
                .route("/flights", web::post().to(create_flight_service))
                .route("/flights/{id}", web::delete().to(delete_flight_service))
                .route("/flights/{id}", web::patch().to(patch_flight_service)),
        )
        .default_service(web::route().to(not_found));
}

fn load_config(config_file: String, secret_config_file: String) -> Result<ServerConfig> {
    let config = Config::builder()
        .add_source(File::with_name(config_file.as_str()))
        .add_source(File::with_name(secret_config_file.as_str()).required(false))
        .build()?;

    let config: ServerConfig = config.try_deserialize()?;
    Ok(config)
}

/// redact_db_url masks the password in a database connection URL of the form
/// `scheme://user:password@host/...` so it can be safely logged. The input is
/// returned unchanged if it cannot be parsed or carries no password.
fn redact_db_url(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_string();
    };
    if parsed.password().is_none() {
        return url.to_string();
    }
    if parsed.set_password(Some("***")).is_err() {
        return url.to_string();
    }
    parsed.into()
}

/// redact_sensitive_fields recursively replaces the values of any object keys
/// whose name suggests a credential (password, secret, token, api key, private
/// key) with `***`, so config parameters can be logged without leaking secrets.
fn redact_sensitive_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                let key = key.to_ascii_lowercase();
                if key.contains("password")
                    || key.contains("secret")
                    || key.contains("token")
                    || key.contains("api_key")
                    || key.contains("private_key")
                {
                    *value = serde_json::Value::String("***".to_owned());
                } else {
                    redact_sensitive_fields(value);
                }
            }
        },
        serde_json::Value::Array(values) => {
            for value in values {
                redact_sensitive_fields(value);
            }
        },
        _ => {},
    }
}

/// log_config_source emits the parameters read from a single config file,
/// tagged with the CLI flag (`source`) it came from so the origin of each
/// value is clear. Credentials embedded in a `database.url` are redacted.
/// `required` controls whether an absent file is reported as a warning
/// (`--config`) or silently skipped (`--secret-config`).
fn log_config_source(config_file: &str, source: &str, required: bool) {
    let parsed = Config::builder()
        .add_source(File::with_name(config_file).required(required))
        .build()
        .and_then(|c| c.try_deserialize::<serde_json::Value>());

    match parsed {
        Ok(mut params) => {
            if params.as_object().is_none_or(|o| o.is_empty()) {
                tracing::debug!(source, config_file, "No parameters found in config file");
                return;
            }
            if let Some(url) = params
                .get("database")
                .and_then(|d| d.get("url"))
                .and_then(|u| u.as_str())
            {
                params["database"]["url"] = serde_json::Value::String(redact_db_url(url));
            }
            redact_sensitive_fields(&mut params);
            tracing::info!(source, config_file, params = %params, "Loaded configuration parameters");
        },
        Err(e) => {
            tracing::warn!(source, config_file, error = %e, "Failed to read config file for logging");
        },
    }
}

async fn read_ca_cert(config: &ServerConfig) -> Result<Option<Vec<u8>>, anyhow::Error> {
    match &config.flight_service.ca_cert {
        Some(path) => {
            tracing::info!(path, "Loading CA certificate for flight service TLS");
            Ok(Some(tokio::fs::read(path).await?))
        },
        None => {
            if config.server.sa_token_file.is_some() {
                tracing::warn!(
                    "sa-token-file is configured but ca-cert is not; \
                     the bearer token will be sent over plaintext HTTP"
                );
            }
            Ok(None)
        },
    }
}

/// Installs the OpenSSL-backed rustls provider as the process default, so
/// that TLS cryptography is performed by the system OpenSSL — and therefore,
/// on a FIPS-mode host, by the validated FIPS provider — rather than by a
/// statically linked Rust implementation.
///
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
    provider_fips: bool,
    config_fips: bool,
    kx_groups: Vec<rustls::NamedGroup>,
}

impl CryptoStatus {
    fn log(&self) {
        tracing::info!(
            provider_fips = self.provider_fips,
            config_fips = self.config_fips,
            kx_groups = ?self.kx_groups,
            "installed OpenSSL-backed rustls crypto provider"
        );
    }
}

fn install_crypto_provider() -> Result<CryptoStatus> {
    use rustls::crypto::CryptoProvider;

    let mut provider = rustls_ossl::default_provider();
    provider.kx_groups.retain(|group| {
        matches!(
            group.name(),
            rustls::NamedGroup::secp256r1 | rustls::NamedGroup::secp384r1 | rustls::NamedGroup::secp521r1
        )
    });

    let kx_groups: Vec<_> = provider.kx_groups.iter().map(|g| g.name()).collect();
    let provider_fips = provider.fips();

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
        provider_fips,
        config_fips,
        kx_groups,
    })
}

#[actix_web::main]
async fn main() -> Result<()> {
    let crypto_status = install_crypto_provider()?;

    let args = CommandLineArgs::parse();
    let config = Arc::new(load_config(args.config.clone(), args.secret_config.clone())?);

    let trace = TraceConfig::from_env();
    let audit = AuditConfig::from_env();
    let tracer_provider = init_tracing_json(SERVICE_NAME, args.json_logs, &trace)?;

    crypto_status.log();

    tracing::info!("Starting DataConnectorHub API service");
    tracing::info!(
        audit_enabled = audit.enabled,
        audit_log_queries = audit.log_queries,
        "Audit logging is {}, query logging is {}",
        if audit.enabled { "enabled" } else { "disabled" },
        if audit.log_queries { "enabled" } else { "disabled" }
    );
    log_config_source(&args.config, "--config", true);
    log_config_source(&args.secret_config, "--secret-config", false);
    log_trace_exporter(&trace);

    let pg_meta_store = Arc::new(
        PgMetaStore::new(
            config.database.clone(),
            config.global_connection_types.tenant_ids.clone(),
        )
        .await?,
    );
    let meta_store: Arc<dyn MetaStore + Send + Sync> = pg_meta_store.clone();

    let secret_store = KubeSecretStore::try_default().await?;

    let flight_ca_cert = read_ca_cert(&config).await?;

    let mut service = ApiService::new(
        meta_store,
        Arc::new(secret_store),
        flight_ca_cert,
        config.server.sa_token_file.clone(),
    );
    if config.auth.enabled {
        tracing::info!("Auth enabled (cache TTL: {}s)", config.auth.cache_ttl_secs);
        let auth_client = kube_utils::auth::KubeAuthClient::try_default(
            std::time::Duration::from_secs(config.auth.cache_ttl_secs),
            // kube-rbac-proxy strips the Authorization header, so the
            // rest-service never reviews request tokens and configures no
            // TokenReview audiences.
            Vec::new(),
        )
        .await?;
        service = service.with_authorizer(std::sync::Arc::new(auth_client));
    } else {
        tracing::warn!("Auth is DISABLED — secret references are not checked against the requester");
    }
    let service = Arc::new(service);

    let metrics_enabled = config.metrics.enabled;
    if metrics_enabled {
        tracing::info!(
            "Prometheus metrics enabled on {}:{}",
            config.metrics.address,
            config.metrics.port
        );
        install_prometheus_recorder()?;
        spawn_metrics_server(config.metrics.address.clone(), config.metrics.port)?;
    } else {
        tracing::info!("Prometheus metrics disabled");
    }

    let server = HttpServer::new(move || {
        let service = service.clone();
        let cors = Cors::default()
            .allow_any_origin()
            .send_wildcard()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .wrap(middleware::Condition::new(
                metrics_enabled,
                middleware::from_fn(observe_http_request),
            ))
            .app_data(web::Data::from(service.clone()))
            .app_data(web::Data::new(audit))
            .app_data(json_config())
            .app_data(query_config())
            .app_data(path_config())
            .configure(move |cfg| api_routes(cfg, service))
    })
    .bind((config.server.address.clone(), config.server.port))?
    .run();

    let server_result = server.await;

    if let Some(provider) = tracer_provider
        && let Err(e) = provider.shutdown()
    {
        tracing::warn!(error = %e, "Failed to flush traces on shutdown");
    }

    server_result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{redact_db_url, redact_sensitive_fields};

    #[test]
    fn test_redact_sensitive_fields_masks_nested_secrets() {
        let mut value = serde_json::json!({
            "server": { "port": 8080 },
            "database": { "url": "postgresql://host/db", "password": "hunter2" },
            "auth": { "api_key": "abc", "Token": "xyz" },
            "connections": [ { "secret": "s3cr3t", "name": "keep" } ]
        });
        redact_sensitive_fields(&mut value);
        assert_eq!(value["server"]["port"], 8080);
        assert_eq!(value["database"]["url"], "postgresql://host/db");
        assert_eq!(value["database"]["password"], "***");
        assert_eq!(value["auth"]["api_key"], "***");
        assert_eq!(value["auth"]["Token"], "***");
        assert_eq!(value["connections"][0]["secret"], "***");
        assert_eq!(value["connections"][0]["name"], "keep");
    }

    #[test]
    fn test_redact_db_url_masks_password() {
        assert_eq!(
            redact_db_url("postgresql://user:secret@localhost:5432/db"),
            "postgresql://user:***@localhost:5432/db"
        );
    }

    #[test]
    fn test_redact_db_url_without_password() {
        assert_eq!(
            redact_db_url("postgresql://user@localhost:5432/db"),
            "postgresql://user@localhost:5432/db"
        );
    }

    #[test]
    fn test_redact_db_url_without_userinfo() {
        assert_eq!(
            redact_db_url("postgresql://localhost:5432/db"),
            "postgresql://localhost:5432/db"
        );
    }

    #[test]
    fn test_redact_db_url_at_in_query_is_not_userinfo() {
        assert_eq!(
            redact_db_url("postgresql://localhost:5432/db?x:y@z"),
            "postgresql://localhost:5432/db?x:y@z"
        );
    }

    #[test]
    fn test_redact_db_url_at_in_path_is_not_userinfo() {
        assert_eq!(
            redact_db_url("postgresql://localhost:5432/d:b@x"),
            "postgresql://localhost:5432/d:b@x"
        );
    }

    #[test]
    fn test_redact_db_url_at_in_fragment_is_not_userinfo() {
        assert_eq!(
            redact_db_url("postgresql://localhost:5432/db#a:b@c"),
            "postgresql://localhost:5432/db#a:b@c"
        );
    }

    #[test]
    fn test_redact_db_url_masks_password_with_at_in_query() {
        assert_eq!(
            redact_db_url("postgresql://user:pass@host:5432/db?a:b@c"),
            "postgresql://user:***@host:5432/db?a:b@c"
        );
    }
}
