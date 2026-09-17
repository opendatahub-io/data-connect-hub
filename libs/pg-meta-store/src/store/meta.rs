use chrono::Utc;
use commons::api::ResourceMetadata;
use commons::api::connection_types::{DataConnectionType, DataConnectionTypeResource, DataConnectionTypeStatus};
use commons::api::connections::{DataConnection, DataConnectionResource, DataConnectionState, DataConnectionStatus};
use commons::api::errors::MetaStoreError;
use commons::api::storage::{MetaStore, MetaStoreReader};
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::{PgPool, Row};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{error, info, warn};
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

const SECRETS_CA_CERT_PATH: &str = "/secrets/postgresql-ca.crt";

/// Number of referencing connection names quoted back in an in-use error.
const REFERENCING_NAME_SAMPLE: i64 = 5;

fn is_sqlstate(e: &sqlx::Error, code: &str) -> bool {
    matches!(e, sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some(code))
}

fn map_sqlx_error(e: sqlx::Error) -> MetaStoreError {
    if is_sqlstate(&e, "23505") {
        return MetaStoreError::Conflict("a resource with the same identity already exists".to_string());
    }
    // On insert and update paths a foreign key violation means the referenced
    // connection type is gone, which is the same condition validate_connection_type
    // reports. Deletes handle 23503 separately, where it means the opposite.
    if is_sqlstate(&e, "23503") {
        return MetaStoreError::UnprocessableEntity("referenced connection type not found".to_string());
    }
    error!("database query failed: {e}");
    MetaStoreError::Query("failed to execute database operation".to_string())
}

// connection_type_in_use_message renders the error returned when a connection type
// cannot be deleted because connections still reference it. An empty `names` slice
// yields a count-only message.
fn connection_type_in_use_message(uid: &str, count: i64, names: &[String]) -> String {
    let subject = if count == 1 {
        "1 connection still references it".to_string()
    } else {
        format!("{count} connections still reference it")
    };
    let detail = if names.is_empty() {
        String::new()
    } else if (names.len() as i64) < count {
        format!(" ({}, ...)", names.join(", "))
    } else {
        format!(" ({})", names.join(", "))
    };
    format!("cannot delete connection type '{uid}': {subject}{detail}; delete the connections first")
}

impl PgMetaStore {
    pub async fn new(config: DatabaseConfig, global_tenant_id: String) -> Result<Self, MetaStoreError> {
        let mut options = PgConnectOptions::from_str(&config.url).map_err(|e| {
            error!("invalid database URL: {e}");
            MetaStoreError::Connection("invalid database URL".to_string())
        })?;

        let has_root_cert = std::env::var("PGSSLROOTCERT").is_ok()
            || url::Url::parse(&config.url).is_ok_and(|u| {
                u.query_pairs()
                    .any(|(k, _)| matches!(k.as_ref(), "sslrootcert" | "ssl-root-cert" | "ssl-ca"))
            });

        if matches!(options.get_ssl_mode(), PgSslMode::VerifyCa | PgSslMode::VerifyFull) && !has_root_cert {
            if Path::new(SECRETS_CA_CERT_PATH).exists() {
                info!("auto-detected CA certificate at {SECRETS_CA_CERT_PATH}");
                options = options.ssl_root_cert(SECRETS_CA_CERT_PATH);
            } else {
                warn!(
                    "sslmode requires CA verification but no certificate found at {SECRETS_CA_CERT_PATH}; \
                     add a postgresql-ca.crt key to the dch-database-config secret"
                );
            }
        }

        let pool = PgPool::connect_with(options).await.map_err(|e| {
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

    // map_delete_connection_type_error converts a failed connection-type delete into
    // a user-facing error. SQLSTATE 23503 means the fk_data_connections_type
    // constraint rejected the delete because connections still reference the type.
    // The referencing rows are counted only here, so a successful delete stays a
    // single statement.
    async fn map_delete_connection_type_error(&self, e: sqlx::Error, tenant_id: &str, uid: &str) -> MetaStoreError {
        if !is_sqlstate(&e, "23503") {
            error!("failed to delete connection type '{uid}': {e}");
            return MetaStoreError::Query("failed to delete connection type".to_string());
        }

        let count = match sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM data_connections WHERE data_connection_type_id = $1",
        )
        .bind(uid)
        .fetch_one(&self.pool)
        .await
        {
            Ok(count) => count,
            Err(e) => {
                error!("failed to count connections referencing connection type '{uid}': {e}");
                return MetaStoreError::Conflict(format!(
                    "cannot delete connection type '{uid}': connections still reference it; delete the connections first"
                ));
            },
        };

        // A global connection type is referenced from tenants the caller may have no
        // visibility into, so only the count is disclosed for those.
        let names = if tenant_id == self.global_tenant_id {
            Vec::new()
        } else {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT data->'resource'->>'name' FROM data_connections \
                 WHERE data_connection_type_id = $1 ORDER BY 1 LIMIT $2",
            )
            .bind(uid)
            .bind(REFERENCING_NAME_SAMPLE)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect()
        };

        MetaStoreError::Conflict(connection_type_in_use_message(uid, count, &names))
    }
}

// deserialize_connection_type deserializes a single stored connection type JSON blob,
// returning the parsed resource or None if it is malformed. A blob that fails to
// deserialize is logged and skipped so a single malformed row does not abort the
// whole listing.
fn deserialize_connection_type(value: serde_json::Value, global_tenant_id: &str) -> Option<DataConnectionTypeResource> {
    match serde_json::from_value::<DataConnectionTypeResource>(value.clone()) {
        Ok(mut dct) => {
            if let Some(tenant) = dct.metadata.tenant_id.clone()
                && tenant == global_tenant_id
            {
                // Discard the tenant field for global connection types
                dct.metadata.tenant_id = None;
            }
            Some(dct)
        },
        Err(e) => {
            let id = value
                .get("metadata")
                .and_then(|m| m.get("id"))
                .and_then(|id| id.as_str())
                .unwrap_or("unknown");
            error!("failed to deserialize connection type {id}: {e}");
            None
        },
    }
}

#[async_trait::async_trait]
impl MetaStoreReader for PgMetaStore {
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
            .filter_map(|row| {
                let value: serde_json::Value = match row.try_get("data") {
                    Ok(value) => value,
                    Err(e) => {
                        error!("failed to read connection type column: {e}");
                        return None;
                    },
                };
                deserialize_connection_type(value, &self.global_tenant_id)
            })
            .collect();

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
}

#[async_trait::async_trait]
impl MetaStore for PgMetaStore {
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
        // The fk_data_connections_type constraint, not this statement, is what keeps
        // connections from being orphaned: the delete blocks on any uncommitted
        // insert referencing this type and then fails with 23503.
        let result = match sqlx::query(
            "DELETE FROM data_connection_types WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2",
        )
        .bind(uid)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        {
            Ok(result) => result,
            Err(e) => return Err(self.map_delete_connection_type_error(e, tenant_id, uid).await),
        };

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

    fn valid_connection_type_json(id: &str, tenant_id: Option<&str>) -> serde_json::Value {
        let mut metadata = serde_json::json!({
            "id": id,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
        });
        if let Some(tenant) = tenant_id {
            metadata["tenant_id"] = serde_json::json!(tenant);
        }
        serde_json::json!({
            "metadata": metadata,
            "resource": {
                "name": "pg",
                "provider": "postgres",
                "credentials_fields": [],
            },
        })
    }

    #[test]
    fn test_deserialize_connection_type_valid() {
        let a = deserialize_connection_type(valid_connection_type_json("a", None), "global");
        let b = deserialize_connection_type(valid_connection_type_json("b", None), "global");
        assert!(a.is_some());
        assert!(b.is_some());
    }

    #[test]
    fn test_deserialize_connection_type_skips_invalid() {
        // Blob is missing the required `resource.name` field.
        let invalid = serde_json::json!({
            "metadata": {
                "id": "bad-1",
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
            },
            "resource": {
                "provider": "postgres",
                "credentials_fields": [],
            },
        });
        assert!(deserialize_connection_type(invalid, "global").is_none());

        let good = deserialize_connection_type(valid_connection_type_json("good-1", None), "global");
        assert_eq!(good.unwrap().metadata.id, "good-1");
    }

    #[test]
    fn test_connection_type_in_use_message_singular() {
        let msg = connection_type_in_use_message("ct-1", 1, &["prod-db".to_string()]);
        assert!(msg.contains("1 connection still references it"), "got: {msg}");
        assert!(msg.contains("(prod-db)"), "got: {msg}");
        assert!(msg.contains("delete the connections first"), "got: {msg}");
    }

    #[test]
    fn test_connection_type_in_use_message_plural() {
        let names = vec!["a".to_string(), "b".to_string()];
        let msg = connection_type_in_use_message("ct-1", 2, &names);
        assert!(msg.contains("2 connections still reference it"), "got: {msg}");
        assert!(msg.contains("(a, b)"), "got: {msg}");
    }

    #[test]
    fn test_connection_type_in_use_message_truncates_sample() {
        // Fewer names than the count means the sample was capped; say so.
        let names = vec!["a".to_string(), "b".to_string()];
        let msg = connection_type_in_use_message("ct-1", 9, &names);
        assert!(msg.contains("(a, b, ...)"), "got: {msg}");
    }

    #[test]
    fn test_connection_type_in_use_message_without_names() {
        // Global connection types disclose the count only, never tenant connection names.
        let msg = connection_type_in_use_message("ct-1", 3, &[]);
        assert!(msg.contains("3 connections still reference it"), "got: {msg}");
        assert!(!msg.contains('('), "expected no name list, got: {msg}");
    }

    #[test]
    fn test_deserialize_connection_type_scrubs_global_tenant() {
        let global = deserialize_connection_type(valid_connection_type_json("g", Some("global")), "global");
        let tenant = deserialize_connection_type(valid_connection_type_json("t", Some("tenant-a")), "global");
        assert_eq!(global.unwrap().metadata.tenant_id, None);
        assert_eq!(tenant.unwrap().metadata.tenant_id, Some("tenant-a".to_string()));
    }
}
