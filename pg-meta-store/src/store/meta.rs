use commons::api::connections::DataConnection;
use commons::api::connections::DataConnectionType;
use commons::api::connections::MetaStore;
use commons::errors::MetaStoreError;
use serde::Deserialize;
use sqlx::{PgPool, Row};

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

pub struct PgMetaStore {
    pool: PgPool,
}

impl PgMetaStore {
    pub async fn new(config: DatabaseConfig) -> Result<Self, MetaStoreError> {
        Ok(Self {
            pool: PgPool::connect(&config.url)
                .await
                .map_err(|e| MetaStoreError::Connection(e.to_string()))?,
        })
    }
}

#[async_trait::async_trait]
impl MetaStore for PgMetaStore {
    async fn get_connection(&self, tenant_id: &str, uid: &str) -> Result<DataConnection, MetaStoreError> {
        let row = sqlx::query("SELECT data FROM data_connections WHERE data->>'id' = $1 AND data->>'tenant_id' = $2")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        let json_value: serde_json::Value = row.get("data");
        serde_json::from_value(json_value).map_err(|e| MetaStoreError::Serialization(e.to_string()))
    }

    async fn get_data_connection_type(&self, _tenant_id: &str, id: &str) -> Result<DataConnectionType, MetaStoreError> {
        // TODO: add tenant_id filter when we have a way to store data connection types per tenant

        let row = sqlx::query("SELECT data FROM data_connection_types WHERE data->>'id' = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        let json_value: serde_json::Value = row.get("data");
        serde_json::from_value(json_value).map_err(|e| MetaStoreError::Serialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_config_deserialize_json() {
        let json = r#"{"url": "postgresql://user:pass@localhost:5432/testdb"}"#;
        let config: DatabaseConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.url, "postgresql://user:pass@localhost:5432/testdb");
    }

    #[test]
    fn test_database_config_deserialize_missing_url() {
        let json = r#"{}"#;
        let result = serde_json::from_str::<DatabaseConfig>(json);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("url"), "expected error about 'url', got: {err}");
    }

    #[test]
    fn test_database_config_debug() {
        let config = DatabaseConfig {
            url: "postgresql://localhost/db".to_string(),
        };
        let debug = format!("{:?}", config);
        assert!(debug.contains("DatabaseConfig"));
        assert!(debug.contains("postgresql://localhost/db"));
    }
}
