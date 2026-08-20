use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use clap::Parser;

use crate::rest::endpoints::*;
use crate::rest::errors::{json_config, path_config, query_config};
use crate::rest::middleware::validate_headers;
use crate::utils::ServerConfig;
use anyhow::Result;
use commons::api::storage::MetaStore;
use config::{Config, File};
use kube_utils::secrets::KubeSecretStore;
use pg_meta_store::store::PgMetaStore;
use std::sync::Arc;
use std::time::Duration;

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
    cfg.route("/api/v1/data/health", web::get().to(health))
        .service(
            web::scope("/api/v1/data").service(
                web::scope("")
                    .wrap(middleware::from_fn(validate_headers))
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
                    .route("/ingestion/{id}", web::get().to(get_ingestion_data)),
            ),
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
/// `scheme://user:password@host/...` so it can be safely logged.
fn redact_db_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];
    // Userinfo, if present, ends at the first '@' before any path/query.
    let Some(at) = rest.find('@') else {
        return url.to_string();
    };
    let userinfo = &rest[..at];
    let Some(colon) = userinfo.find(':') else {
        return url.to_string();
    };
    format!("{}{}:***{}", &url[..authority_start], &userinfo[..colon], &rest[at..])
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
            tracing::info!(source, config_file, params = %params, "Loaded configuration parameters");
        },
        Err(e) => {
            tracing::warn!(source, config_file, error = %e, "Failed to read config file for logging");
        },
    }
}

#[actix_web::main]
async fn main() -> Result<()> {
    let args = CommandLineArgs::parse();
    let config = load_config(args.config.clone(), args.secret_config.clone())?;

    commons::utils::init_tracing(args.json_logs);
    tracing::info!("Starting DataConnectorHub API service");
    log_config_source(&args.config, "--config", true);
    log_config_source(&args.secret_config, "--secret-config", false);

    let pg_meta_store =
        Arc::new(PgMetaStore::new(config.database, config.global_connection_types.tenant_id.clone()).await?);
    let meta_store: Arc<dyn MetaStore + Send + Sync> = pg_meta_store.clone();

    let secret_store = KubeSecretStore::try_default(Duration::from_secs(300)).await?;

    let service = Arc::new(ApiService::new(meta_store, Arc::new(secret_store)));

    HttpServer::new(move || {
        let service = service.clone();
        let cors = Cors::default()
            .allow_any_origin()
            .send_wildcard()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(web::Data::from(service.clone()))
            .app_data(json_config())
            .app_data(query_config())
            .app_data(path_config())
            .configure(move |cfg| api_routes(cfg, service))
    })
    .bind((config.server.address, config.server.port))?
    .run()
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::redact_db_url;

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
}
