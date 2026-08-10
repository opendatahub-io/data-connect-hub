use chrono::Utc;
use commons::api::ResourceMetadata;
use commons::api::connections::Admin;
use commons::api::connections::{
    DataConnection, DataConnectionResource, DataConnectionType, DataConnectionTypeResource, MetaStore,
};
use commons::api::errors::MetaStoreError;
use serde::Deserialize;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

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

    async fn can_store(data_connection: &DataConnection) -> Result<(), MetaStoreError> {
        if let Some(Admin::Secret { .. }) = &data_connection.admin {
            return Err(MetaStoreError::Validation(
                "A plain secret cannot be stored in the database, use a secret reference instead".to_string(),
            ));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl MetaStore for PgMetaStore {
    async fn get_data_connection(&self, tenant_id: &str, uid: &str) -> Result<DataConnectionResource, MetaStoreError> {
        let row = sqlx::query("SELECT data FROM data_connections WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => MetaStoreError::ResourceNotFound(format!("Data connection '{uid}' not found for tenant '{tenant_id}'")),
                e => MetaStoreError::Query(e.to_string()),
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| MetaStoreError::Query(e.to_string()))?;
        serde_json::from_value(json_value).map_err(|e| MetaStoreError::Deserialization(e.to_string()))
    }

    async fn create_data_connection(
        &self,
        tenant_id: &str,
        data_connection: DataConnection,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        Self::can_store(&data_connection).await?;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resource = DataConnectionResource {
            metadata: ResourceMetadata {
                id: Uuid::new_v4().to_string(),
                tenant_id: tenant_id.to_string(),
                created_at: now.clone(),
                updated_at: now,
            },
            resource: data_connection,
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| MetaStoreError::Serialization(e.to_string()))?;

        sqlx::query("INSERT INTO data_connections (data) VALUES ($1)")
            .bind(&json_value)
            .execute(&self.pool)
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        Ok(resource)
    }

    async fn update_data_connection(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnection) -> Result<DataConnection, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        let row = sqlx::query("SELECT data FROM data_connections WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2 FOR UPDATE")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => MetaStoreError::ResourceNotFound(format!("Data connection '{uid}' not found for tenant '{tenant_id}'")),
                e => MetaStoreError::Query(e.to_string()),
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| MetaStoreError::Query(e.to_string()))?;
        let existing: DataConnectionResource =
            serde_json::from_value(json_value).map_err(|e| MetaStoreError::Deserialization(e.to_string()))?;

        let data_connection = update_fn(existing.resource)?;

        Self::can_store(&data_connection).await?;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let resource = DataConnectionResource {
            metadata: ResourceMetadata {
                updated_at: now,
                ..existing.metadata
            },
            resource: data_connection,
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| MetaStoreError::Serialization(e.to_string()))?;

        sqlx::query("UPDATE data_connections SET data = $1 WHERE data->'metadata'->>'id' = $2 AND data->'metadata'->>'tenant_id' = $3")
            .bind(&json_value)
            .bind(uid)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        tx.commit().await.map_err(|e| MetaStoreError::Query(e.to_string()))?;

        Ok(resource)
    }

    async fn delete_data_connection(&self, tenant_id: &str, uid: &str) -> Result<(), MetaStoreError> {
        let result = sqlx::query(
            "DELETE FROM data_connections WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2",
        )
        .bind(uid)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(MetaStoreError::ResourceNotFound(format!(
                "Data connection '{uid}' not found for tenant '{tenant_id}'"
            )));
        }

        Ok(())
    }

    async fn get_data_connection_type(
        &self,
        _tenant_id: &str,
        id: &str,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        // TODO: add tenant_id filter when we have a way to store data connection types per tenant

        let row = sqlx::query("SELECT data FROM data_connection_types WHERE data->'metadata'->>'id' = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    MetaStoreError::ResourceNotFound(format!("connection type '{id}' not found"))
                },
                e => MetaStoreError::Query(e.to_string()),
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| MetaStoreError::Query(e.to_string()))?;
        serde_json::from_value(json_value).map_err(|e| MetaStoreError::Deserialization(e.to_string()))
    }

    async fn create_data_connection_type(
        &self,
        tenant_id: &str,
        data_connection_type: DataConnectionType,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resource = DataConnectionTypeResource {
            metadata: ResourceMetadata {
                id: Uuid::new_v4().to_string(),
                tenant_id: tenant_id.to_string(),
                created_at: now.clone(),
                updated_at: now,
            },
            resource: data_connection_type,
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| MetaStoreError::Serialization(e.to_string()))?;

        sqlx::query("INSERT INTO data_connection_types (data) VALUES ($1)")
            .bind(&json_value)
            .execute(&self.pool)
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        Ok(resource)
    }

    async fn update_data_connection_type(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnectionType) -> Result<DataConnectionType, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        let row = sqlx::query("SELECT data FROM data_connection_types WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2 FOR UPDATE")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => MetaStoreError::ResourceNotFound(format!("Connection type '{uid}' not found for tenant '{tenant_id}'")),
                e => MetaStoreError::Query(e.to_string()),
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| MetaStoreError::Query(e.to_string()))?;
        let existing: DataConnectionTypeResource =
            serde_json::from_value(json_value).map_err(|e| MetaStoreError::Deserialization(e.to_string()))?;

        let data_connection_type = update_fn(existing.resource)?;
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let resource = DataConnectionTypeResource {
            metadata: ResourceMetadata {
                updated_at: now,
                ..existing.metadata
            },
            resource: data_connection_type,
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| MetaStoreError::Serialization(e.to_string()))?;

        sqlx::query("UPDATE data_connection_types SET data = $1 WHERE data->'metadata'->>'id' = $2 AND data->'metadata'->>'tenant_id' = $3")
            .bind(&json_value)
            .bind(uid)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        tx.commit().await.map_err(|e| MetaStoreError::Query(e.to_string()))?;

        Ok(resource)
    }

    async fn delete_data_connection_type(&self, tenant_id: &str, uid: &str) -> Result<(), MetaStoreError> {
        let result = sqlx::query(
            "DELETE FROM data_connection_types WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2",
        )
        .bind(uid)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| MetaStoreError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(MetaStoreError::ResourceNotFound(format!(
                "Data connection type '{uid}' not found for tenant '{tenant_id}'"
            )));
        }

        Ok(())
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
