use chrono::Utc;
use commons::api::ResourceMetadata;
use commons::api::connection_types::{DataConnectionType, DataConnectionTypeResource, DataConnectionTypeStatus};
use commons::api::connections::{DataConnection, DataConnectionResource, DataConnectionState, DataConnectionStatus};
use commons::api::errors::MetaStoreError;
use commons::api::storage::MetaStore;
use serde::Deserialize;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tracing::error;
use uuid::Uuid;

use commons::api::ResourceList;

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
}

pub struct PgMetaStore {
    pool: PgPool,
    global_tenant_id: String,
}

fn map_sqlx_error(e: sqlx::Error) -> MetaStoreError {
    if let sqlx::Error::Database(ref db_err) = e
        && db_err.code().as_deref() == Some("23505")
    {
        return MetaStoreError::Conflict("a resource with the same identity already exists".to_string());
    }
    error!("database query failed: {e}");
    MetaStoreError::Query("failed to execute database operation".to_string())
}

impl PgMetaStore {
    pub async fn new(config: DatabaseConfig, global_tenant_id: String) -> Result<Self, MetaStoreError> {
        let pool = PgPool::connect(&config.url).await.map_err(|e| {
            error!("failed to connect to database: {e}");
            MetaStoreError::Connection("failed to connect to the metadata database".to_string())
        })?;

        Self::init_schema(&pool).await?;

        Ok(Self { pool, global_tenant_id })
    }

    async fn init_schema(pool: &PgPool) -> Result<(), MetaStoreError> {
        sqlx::raw_sql(include_str!("../../schema/connections.sql"))
            .execute(pool)
            .await
            .map_err(|e| {
                error!("schema initialization failed: {e}");
                MetaStoreError::Query("failed to initialize database schema".to_string())
            })?;
        Ok(())
    }

    async fn validate_connection_type<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
        executor: E,
        global_tenant_id: &str,
        tenant_id: &str,
        connection_type_id: &str,
    ) -> Result<(), MetaStoreError> {
        sqlx::query("SELECT 1 FROM data_connection_types WHERE data->'metadata'->>'id' = $1 AND (data->'metadata'->>'tenant_id' = $2 OR data->'metadata'->>'tenant_id' = $3) FOR SHARE")
            .bind(connection_type_id)
            .bind(tenant_id)
            .bind(global_tenant_id)
            .fetch_one(executor)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    MetaStoreError::UnprocessableEntity(format!("connection type '{connection_type_id}' not found"))
                }
                e => {
                    error!("failed to validate connection type '{connection_type_id}': {e}");
                    MetaStoreError::Query("failed to validate connection type".to_string())
                }
            })?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl MetaStore for PgMetaStore {
    async fn get_data_connections(
        &self,
        tenant_id: &str,
    ) -> Result<ResourceList<DataConnectionResource>, MetaStoreError> {
        let rows = sqlx::query("SELECT data FROM data_connections WHERE data->'metadata'->>'tenant_id' = $1")
            .bind(tenant_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!("failed to list data connections: {e}");
                MetaStoreError::Query("failed to list data connections".to_string())
            })?;

        let items: Vec<DataConnectionResource> = rows
            .iter()
            .map(|row| {
                let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
                    error!("failed to read data connection column: {e}");
                    MetaStoreError::Query("failed to read data connection".to_string())
                })?;
                serde_json::from_value(json_value).map_err(|e| {
                    error!("failed to deserialize data connection: {e}");
                    MetaStoreError::Deserialization("failed to deserialize data connection".to_string())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ResourceList {
            total_count: items.len(),
            items,
        })
    }

    async fn get_data_connection(&self, tenant_id: &str, uid: &str) -> Result<DataConnectionResource, MetaStoreError> {
        let row = sqlx::query("SELECT data FROM data_connections WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => MetaStoreError::ResourceNotFound(format!("data connection '{uid}' not found")),
                e => {
                    error!("failed to get data connection '{uid}': {e}");
                    MetaStoreError::Query("failed to retrieve data connection".to_string())
                }
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read data connection column: {e}");
            MetaStoreError::Query("failed to read data connection".to_string())
        })?;
        serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize data connection: {e}");
            MetaStoreError::Deserialization("failed to deserialize data connection".to_string())
        })
    }

    async fn create_data_connection(
        &self,
        tenant_id: &str,
        data_connection: &DataConnection,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("failed to begin transaction: {e}");
            MetaStoreError::Query("failed to create data connection".to_string())
        })?;

        Self::validate_connection_type(
            &mut *tx,
            &self.global_tenant_id,
            tenant_id,
            &data_connection.data_connection_type_id,
        )
        .await?;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resource = DataConnectionResource {
            metadata: ResourceMetadata {
                id: Uuid::new_v4().to_string(),
                tenant_id: Some(tenant_id.to_string()),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
            resource: data_connection.clone(),
            status: DataConnectionStatus {
                state: DataConnectionState::NotReady,
                message: None,
                updated_at: Some(now.clone()),
            },
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| {
            error!("failed to serialize data connection: {e}");
            MetaStoreError::Serialization("failed to serialize data connection".to_string())
        })?;

        sqlx::query("INSERT INTO data_connections (data) VALUES ($1)")
            .bind(&json_value)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(|e| {
            error!("failed to commit transaction: {e}");
            MetaStoreError::Query("failed to create data connection".to_string())
        })?;

        Ok(resource)
    }

    async fn update_data_connection(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnection) -> Result<DataConnection, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("failed to begin transaction: {e}");
            MetaStoreError::Query("failed to update data connection".to_string())
        })?;

        let row = sqlx::query("SELECT data FROM data_connections WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2 FOR UPDATE")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => MetaStoreError::ResourceNotFound(format!("data connection '{uid}' not found")),
                e => {
                    error!("failed to get data connection '{uid}' for update: {e}");
                    MetaStoreError::Query("failed to update data connection".to_string())
                }
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read data connection column: {e}");
            MetaStoreError::Query("failed to read data connection".to_string())
        })?;
        let existing: DataConnectionResource = serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize data connection: {e}");
            MetaStoreError::Deserialization("failed to deserialize data connection".to_string())
        })?;

        let data_connection = update_fn(existing.resource)?;

        Self::validate_connection_type(
            &mut *tx,
            &self.global_tenant_id,
            tenant_id,
            &data_connection.data_connection_type_id,
        )
        .await?;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let resource = DataConnectionResource {
            metadata: ResourceMetadata {
                updated_at: now,
                ..existing.metadata
            },
            resource: data_connection,

            // TODO: For now we preserve the same status but since the connection changed we'll need to revalidate the connection and set the connection statud.
            status: existing.status.clone(),
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| {
            error!("failed to serialize data connection: {e}");
            MetaStoreError::Serialization("failed to serialize data connection".to_string())
        })?;

        sqlx::query("UPDATE data_connections SET data = $1 WHERE data->'metadata'->>'id' = $2 AND data->'metadata'->>'tenant_id' = $3")
            .bind(&json_value)
            .bind(uid)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!("failed to update data connection '{uid}': {e}");
                MetaStoreError::Query("failed to update data connection".to_string())
            })?;

        tx.commit().await.map_err(|e| {
            error!("failed to commit transaction: {e}");
            MetaStoreError::Query("failed to update data connection".to_string())
        })?;

        Ok(resource)
    }

    async fn update_data_connection_status(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnectionStatus) -> Result<DataConnectionStatus, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionResource, MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("failed to begin transaction: {e}");
            MetaStoreError::Query("failed to update data connection status".to_string())
        })?;

        let row = sqlx::query("SELECT data FROM data_connections WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2 FOR UPDATE")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    MetaStoreError::ResourceNotFound(format!("data connection '{uid}' not found"))
                },
                e => {
                    error!("failed to get data connection '{uid}' for status update: {e}");
                    MetaStoreError::Query("failed to update data connection status".to_string())
                },
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read data connection column: {e}");
            MetaStoreError::Query("failed to read data connection".to_string())
        })?;
        let existing: DataConnectionResource = serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize data connection: {e}");
            MetaStoreError::Deserialization("failed to deserialize data connection".to_string())
        })?;

        let status = update_fn(existing.status)?;
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resource = DataConnectionResource {
            metadata: ResourceMetadata {
                updated_at: now,
                ..existing.metadata
            },
            resource: existing.resource,
            status,
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| {
            error!("failed to serialize data connection: {e}");
            MetaStoreError::Serialization("failed to serialize data connection".to_string())
        })?;

        sqlx::query("UPDATE data_connections SET data = $1 WHERE data->'metadata'->>'id' = $2 AND data->'metadata'->>'tenant_id' = $3")
            .bind(&json_value)
            .bind(uid)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!("failed to update data connection status '{uid}': {e}");
                MetaStoreError::Query("failed to update data connection status".to_string())
            })?;

        tx.commit().await.map_err(|e| {
            error!("failed to commit transaction: {e}");
            MetaStoreError::Query("failed to update data connection status".to_string())
        })?;

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
        .map_err(|e| {
            error!("failed to delete data connection '{uid}': {e}");
            MetaStoreError::Query("failed to delete data connection".to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(MetaStoreError::ResourceNotFound(format!(
                "data connection '{uid}' not found"
            )));
        }

        Ok(())
    }

    async fn get_all_data_connection_types(&self) -> Result<ResourceList<DataConnectionTypeResource>, MetaStoreError> {
        let rows = sqlx::query("SELECT data FROM data_connection_types")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!("failed to list connection types: {e}");
                MetaStoreError::Query("failed to list connection types".to_string())
            })?;

        let items: Vec<DataConnectionTypeResource> = rows
            .iter()
            .map(|row| {
                let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
                    error!("failed to read connection type column: {e}");
                    MetaStoreError::Query("failed to read connection type".to_string())
                })?;
                let mut dct: DataConnectionTypeResource = serde_json::from_value(json_value).map_err(|e| {
                    error!("failed to deserialize connection type: {e}");
                    MetaStoreError::Deserialization(
                        "failed to deserialize connection type; see service logs for details".to_string(),
                    )
                })?;
                if let Some(tenant) = dct.metadata.tenant_id.clone()
                    && tenant == self.global_tenant_id
                {
                    // Discard the tenant field for global connection types
                    dct.metadata.tenant_id = None;
                }
                Ok(dct)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ResourceList {
            total_count: items.len(),
            items,
        })
    }
    async fn get_data_connection_types(
        &self,
        tenant_id: &str,
    ) -> Result<ResourceList<DataConnectionTypeResource>, MetaStoreError> {
        let rows = sqlx::query("SELECT data FROM data_connection_types WHERE data->'metadata'->>'tenant_id' = $1 OR data->'metadata'->>'tenant_id' = $2")
            .bind(tenant_id)
            .bind(&self.global_tenant_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!("failed to list connection types: {e}");
                MetaStoreError::Query("failed to list connection types".to_string())
            })?;

        let items: Vec<DataConnectionTypeResource> = rows
            .iter()
            .map(|row| {
                let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
                    error!("failed to read connection type column: {e}");
                    MetaStoreError::Query("failed to read connection type".to_string())
                })?;
                let mut dct: DataConnectionTypeResource = serde_json::from_value(json_value).map_err(|e| {
                    error!("failed to deserialize connection type: {e}");
                    MetaStoreError::Deserialization(
                        "failed to deserialize connection type; see service logs for details".to_string(),
                    )
                })?;
                if let Some(tenant) = dct.metadata.tenant_id.clone()
                    && tenant == self.global_tenant_id
                {
                    // Discard the tenant field for global connection types
                    dct.metadata.tenant_id = None;
                }
                Ok(dct)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ResourceList {
            total_count: items.len(),
            items,
        })
    }

    async fn get_data_connection_type(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        let row = sqlx::query("SELECT data FROM data_connection_types WHERE data->'metadata'->>'id' = $1 AND (data->'metadata'->>'tenant_id' = $2 OR data->'metadata'->>'tenant_id' = $3)")
            .bind(id)
            .bind(tenant_id)
            .bind(&self.global_tenant_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    MetaStoreError::ResourceNotFound(format!("connection type '{id}' not found"))
                },
                e => {
                    error!("failed to get connection type '{id}': {e}");
                    MetaStoreError::Query("failed to retrieve connection type".to_string())
                }
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read connection type column: {e}");
            MetaStoreError::Query("failed to read connection type".to_string())
        })?;
        serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize connection type: {e}");
            MetaStoreError::Deserialization(
                "failed to deserialize connection type; see service logs for details".to_string(),
            )
        })
    }

    async fn create_data_connection_type(
        &self,
        tenant_id: &str,
        data_connection_type: &DataConnectionType,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resource = DataConnectionTypeResource {
            metadata: ResourceMetadata {
                id: Uuid::new_v4().to_string(),
                tenant_id: Some(tenant_id.to_string()),
                created_at: now.clone(),
                updated_at: now,
            },
            resource: data_connection_type.clone(),
            status: Default::default(),
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| {
            error!("failed to serialize connection type: {e}");
            MetaStoreError::Serialization("failed to serialize connection type".to_string())
        })?;

        sqlx::query("INSERT INTO data_connection_types (data) VALUES ($1)")
            .bind(&json_value)
            .execute(&self.pool)
            .await
            .map_err(|e| match map_sqlx_error(e) {
                MetaStoreError::Conflict(_) => MetaStoreError::Conflict(format!(
                    "a connection type named '{}' already exists for this tenant",
                    data_connection_type.name
                )),
                other => other,
            })?;

        Ok(resource)
    }

    async fn update_data_connection_type(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnectionType) -> Result<DataConnectionType, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("failed to begin transaction: {e}");
            MetaStoreError::Query("failed to update connection type".to_string())
        })?;

        let row = sqlx::query("SELECT data FROM data_connection_types WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2 FOR UPDATE")
            .bind(uid)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => MetaStoreError::ResourceNotFound(format!("connection type '{uid}' not found")),
                e => {
                    error!("failed to get connection type '{uid}' for update: {e}");
                    MetaStoreError::Query("failed to update connection type".to_string())
                }
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read connection type column: {e}");
            MetaStoreError::Query("failed to read connection type".to_string())
        })?;
        let existing: DataConnectionTypeResource = serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize connection type: {e}");
            MetaStoreError::Deserialization(
                "failed to deserialize connection type; see service logs for details".to_string(),
            )
        })?;

        let data_connection_type = update_fn(existing.resource)?;
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let resource = DataConnectionTypeResource {
            metadata: ResourceMetadata {
                updated_at: now,
                ..existing.metadata
            },
            resource: data_connection_type,
            status: existing.status.clone(),
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| {
            error!("failed to serialize connection type: {e}");
            MetaStoreError::Serialization("failed to serialize connection type".to_string())
        })?;

        sqlx::query("UPDATE data_connection_types SET data = $1 WHERE data->'metadata'->>'id' = $2 AND data->'metadata'->>'tenant_id' = $3")
            .bind(&json_value)
            .bind(uid)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!("failed to update connection type '{uid}': {e}");
                MetaStoreError::Query("failed to update connection type".to_string())
            })?;

        tx.commit().await.map_err(|e| {
            error!("failed to commit transaction: {e}");
            MetaStoreError::Query("failed to update connection type".to_string())
        })?;

        Ok(resource)
    }

    async fn update_data_connection_type_status(
        &self,
        uid: &str,
        update_fn: Arc<
            dyn Fn(DataConnectionTypeStatus) -> Result<DataConnectionTypeStatus, MetaStoreError> + Send + Sync,
        >,
    ) -> Result<DataConnectionTypeResource, MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("failed to begin transaction: {e}");
            MetaStoreError::Query("failed to update connection type status".to_string())
        })?;

        let row = sqlx::query("SELECT data FROM data_connection_types WHERE data->'metadata'->>'id' = $1 FOR UPDATE")
            .bind(uid)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    MetaStoreError::ResourceNotFound(format!("connection type '{uid}' not found"))
                },
                e => {
                    error!("failed to get connection type '{uid}' for status update: {e}");
                    MetaStoreError::Query("failed to update connection type status".to_string())
                },
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read connection type column: {e}");
            MetaStoreError::Query("failed to read connection type".to_string())
        })?;
        let existing: DataConnectionTypeResource = serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize connection type: {e}");
            MetaStoreError::Deserialization(
                "failed to deserialize connection type; see service logs for details".to_string(),
            )
        })?;

        let status = update_fn(existing.status)?;
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resource = DataConnectionTypeResource {
            metadata: ResourceMetadata {
                updated_at: now,
                ..existing.metadata
            },
            resource: existing.resource,
            status,
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| {
            error!("failed to serialize connection type: {e}");
            MetaStoreError::Serialization("failed to serialize connection type".to_string())
        })?;

        sqlx::query("UPDATE data_connection_types SET data = $1 WHERE data->'metadata'->>'id' = $2")
            .bind(&json_value)
            .bind(uid)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!("failed to update connection type status '{uid}': {e}");
                MetaStoreError::Query("failed to update connection type status".to_string())
            })?;

        tx.commit().await.map_err(|e| {
            error!("failed to commit transaction: {e}");
            MetaStoreError::Query("failed to update connection type status".to_string())
        })?;

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
        .map_err(|e| {
            error!("failed to delete connection type '{uid}': {e}");
            MetaStoreError::Query("failed to delete connection type".to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(MetaStoreError::ResourceNotFound(format!(
                "connection type '{uid}' not found"
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
