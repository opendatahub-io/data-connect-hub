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
use url::Url;

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
