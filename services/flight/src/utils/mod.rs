use pg_meta_store::store::DatabaseConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Server {
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct IngestionCachePools {
    pub max_capacity: u64,
    pub ttl_secs: u64,
    pub idle_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub server: Server,
    pub database: DatabaseConfig,
    pub ingestion_cache_pools: IngestionCachePools,
}
