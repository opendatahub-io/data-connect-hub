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
pub struct QueryConfig {
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    512
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            batch_size: default_batch_size(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "default_token_review_audiences")]
    pub token_review_audiences: Vec<String>,
}

fn default_cache_ttl_secs() -> u64 {
    300
}

fn default_token_review_audiences() -> Vec<String> {
    vec!["https://kubernetes.default.svc".to_string()]
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cache_ttl_secs: default_cache_ttl_secs(),
            token_review_audiences: default_token_review_audiences(),
        }
    }
}

impl QueryConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.batch_size == 0 {
            return Err("query.batch_size must be greater than 0".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub server: Server,
    pub database: DatabaseConfig,
    pub ingestion_cache_pools: IngestionCachePools,
    #[serde(default)]
    pub query: QueryConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}
