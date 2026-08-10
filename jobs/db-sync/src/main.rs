use anyhow::Result;
use clap::Parser;
use commons::api::connections::{DataConnectionType, MetaStore};
use config::{Config, File};
use pg_meta_store::store::{DatabaseConfig, PgMetaStore};
use serde::Deserialize;
use std::fs;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(author, version, about = "Sync default connection types to the database")]
struct Args {
    /// Config file
    #[arg(short, long, default_value = "config/config.toml")]
    config: String,

    /// Optional secret config file merged on top of `config`
    #[arg(long, default_value = "/secrets/secret-config.toml")]
    secret_config: String,

    /// Folder containing default connection type YAML files
    #[arg(short, long, default_value = "config/connection-types")]
    folder: String,

    /// Enable JSON logs
    #[arg(short, long, default_value = "false")]
    json_logs: bool,
}

#[derive(Debug, Deserialize)]
struct JobConfig {
    database: DatabaseConfig,
}

fn load_default_connection_types(folder: &str) -> Result<Vec<DataConnectionType>> {
    let mut types = Vec::new();
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "yaml" || ext == "yml") {
            let content = fs::read_to_string(&path)?;
            let ct: DataConnectionType = serde_yaml::from_str(&content)?;
            types.push(ct);
        }
    }
    Ok(types)
}

async fn sync_default_connection_types(meta_store: &Arc<dyn MetaStore + Send + Sync>, folder: &str) -> Result<()> {
    let defaults = load_default_connection_types(folder)?;
    tracing::info!("Loaded {} default connection types", defaults.len());

    for ct in &defaults {
        meta_store.set_default_connection_type(ct).await?;
        tracing::info!("Synced default connection type: {}", ct.name);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    commons::utils::init_tracing(args.json_logs);

    let config = Config::builder()
        .add_source(File::with_name(&args.config))
        .add_source(File::with_name(&args.secret_config).required(false))
        .build()?;

    let job_config: JobConfig = config.try_deserialize()?;
    let db_config = job_config.database;
    let meta_store: Arc<dyn MetaStore + Send + Sync> = Arc::new(PgMetaStore::new(db_config).await?);

    sync_default_connection_types(&meta_store, &args.folder).await?;

    tracing::info!("Default connection types sync complete");
    Ok(())
}
