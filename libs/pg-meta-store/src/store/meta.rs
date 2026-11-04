use chrono::Utc;
use commons::api::ResourceMetadata;
use commons::api::connection_types::{DataConnectionType, DataConnectionTypeResource, DataConnectionTypeStatus};
use commons::api::connections::{DataConnection, DataConnectionResource, DataConnectionState, DataConnectionStatus};
use commons::api::errors::MetaStoreError;
use commons::api::flight_discovery::{FlightService, FlightServiceResource};
use commons::api::storage::{FlightDiscoveryStore, MetaStore, MetaStoreReader};
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

/// `dch_settings` key holding the global connection type tenant that the
/// stored records were last written under.
const GLOBAL_TENANT_SETTING: &str = "global_connection_types.tenant_id";

/// Advisory lock serializing [`PgMetaStore::sync_global_tenant`] across
/// replicas starting at the same time.
const GLOBAL_TENANT_SYNC_LOCK: i64 = 0x4443_485f_4754_4e54; // "DCH_GTNT"

/// Moves global connection types from the previous global tenant ($1) to the
/// new one ($2). A type whose name already exists in the new tenant is left in
/// place, since the (name, tenant) pair is unique.
const MIGRATE_GLOBAL_CONNECTION_TYPES: &str = "UPDATE data_connection_types AS t \
     SET data = jsonb_set(t.data, '{metadata,tenant_id}', to_jsonb($2::text)) \
     WHERE t.data->'metadata'->>'tenant_id' = $1 \
     AND NOT EXISTS ( \
         SELECT 1 FROM data_connection_types AS n \
         WHERE n.data->'metadata'->>'tenant_id' = $2 \
         AND n.data->'resource'->>'name' = t.data->'resource'->>'name')";

/// Outcome of [`PgMetaStore::sync_global_tenant`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalTenantSync {
    /// No tenant was recorded; the configured tenant is now recorded.
    Initialized,
    /// The configured tenant matches the recorded one.
    Unchanged,
    /// The global tenant changed and its connection types were moved.
    Migrated {
        previous: String,
        /// Connection types moved to the new global tenant.
        moved: u64,
        /// Connection types left in the previous tenant because a type with
        /// the same name already exists in the new one.
        conflicts: i64,
    },
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

    /// Reconciles the stored global connection type tenant with the
    /// configured one.
    ///
    /// The global tenant is the RHOAI global namespace when one is
    /// designated, otherwise the DataConnectService namespace, so it changes
    /// when an administrator designates, moves, or clears the global
    /// namespace. Global connection types are stored under the global tenant,
    /// so without this migration they would turn into tenant-local types of
    /// the previous namespace and connections in other tenants could no
    /// longer resolve them. Connections are not moved: they stay in their own
    /// tenant and keep referencing the types by id.
    pub async fn sync_global_tenant(&self) -> Result<GlobalTenantSync, MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(GLOBAL_TENANT_SYNC_LOCK)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        let previous: Option<String> = sqlx::query_scalar("SELECT value FROM dch_settings WHERE key = $1")
            .bind(GLOBAL_TENANT_SETTING)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        let outcome = match previous {
            None => {
                sqlx::query("INSERT INTO dch_settings (key, value) VALUES ($1, $2)")
                    .bind(GLOBAL_TENANT_SETTING)
                    .bind(&self.global_tenant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_sqlx_error)?;
                GlobalTenantSync::Initialized
            },
            Some(previous) if previous == self.global_tenant_id => GlobalTenantSync::Unchanged,
            Some(previous) => {
                let moved = sqlx::query(MIGRATE_GLOBAL_CONNECTION_TYPES)
                    .bind(&previous)
                    .bind(&self.global_tenant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_sqlx_error)?
                    .rows_affected();

                let conflicts: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM data_connection_types WHERE data->'metadata'->>'tenant_id' = $1",
                )
                .bind(&previous)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx_error)?;

                sqlx::query("UPDATE dch_settings SET value = $2 WHERE key = $1")
                    .bind(GLOBAL_TENANT_SETTING)
                    .bind(&self.global_tenant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_sqlx_error)?;

                GlobalTenantSync::Migrated {
                    previous,
                    moved,
                    conflicts,
                }
            },
        };

        tx.commit().await.map_err(map_sqlx_error)?;

        match &outcome {
            GlobalTenantSync::Initialized => {
                info!(tenant = %self.global_tenant_id, "recorded global connection type tenant");
            },
            GlobalTenantSync::Unchanged => {},
            GlobalTenantSync::Migrated {
                previous,
                moved,
                conflicts,
            } => {
                info!(
                    previous = %previous,
                    tenant = %self.global_tenant_id,
                    moved,
                    "moved global connection types to the new global tenant"
                );
                if *conflicts > 0 {
                    warn!(
                        previous = %previous,
                        tenant = %self.global_tenant_id,
                        conflicts,
                        "connection types left in the previous global tenant because the new tenant already has types with the same names; they are no longer global"
                    );
                }
            },
        }
        Ok(outcome)
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "SELECT",
        db.collection.name = "data_connection_types",
        tenant.id = %tenant_id,
        resource.id = %connection_type_id,
        )
    )]
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

// connection_type_in_use_message renders the error returned when a connection type
// cannot be deleted because connections still reference it. Only the number of
// referencing connections is disclosed: a global connection type may be referenced
// from tenants the caller cannot otherwise see.
fn connection_type_in_use_message(uid: &str, count: i64) -> String {
    let subject = if count == 1 {
        "1 connection still references it".to_string()
    } else {
        format!("{count} connections still reference it")
    };
    format!("cannot delete connection type '{uid}': {subject}; delete the connections first")
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
    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "SELECT",
        db.collection.name = "data_connections",
        tenant.id = %tenant_id,
        row_count = tracing::field::Empty,
        )
    )]
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

        tracing::Span::current().record("row_count", items.len());

        Ok(ResourceList {
            total_count: items.len(),
            items,
        })
    }

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "SELECT",
        db.collection.name = "data_connections",
        tenant.id = %tenant_id,
        resource.id = %uid,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "SELECT",
        db.collection.name = "data_connection_types",
        tenant.id = %tenant_id,
        row_count = tracing::field::Empty,
        )
    )]
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

        tracing::Span::current().record("row_count", items.len());

        Ok(ResourceList {
            total_count: items.len(),
            items,
        })
    }

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "SELECT",
        db.collection.name = "data_connection_types",
        tenant.id = %tenant_id,
        resource.id = %id,
        )
    )]
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
    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "INSERT",
        db.collection.name = "data_connections",
        tenant.id = %tenant_id,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "UPDATE",
        db.collection.name = "data_connections",
        tenant.id = %tenant_id,
        resource.id = %uid,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "UPDATE",
        db.collection.name = "data_connections",
        tenant.id = %tenant_id,
        resource.id = %uid,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "DELETE",
        db.collection.name = "data_connections",
        tenant.id = %tenant_id,
        resource.id = %uid,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "SELECT",
        db.collection.name = "data_connection_types",
        row_count = tracing::field::Empty,
        )
    )]
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

        tracing::Span::current().record("row_count", items.len());

        Ok(ResourceList {
            total_count: items.len(),
            items,
        })
    }

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "INSERT",
        db.collection.name = "data_connection_types",
        tenant.id = %tenant_id,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "UPDATE",
        db.collection.name = "data_connection_types",
        tenant.id = %tenant_id,
        resource.id = %uid,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "UPDATE",
        db.collection.name = "data_connection_types",
        resource.id = %uid,
        )
    )]
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

    #[tracing::instrument(
        skip_all,
        fields(
        db.system = "postgresql",
        db.operation = "DELETE",
        db.collection.name = "data_connection_types",
        tenant.id = %tenant_id,
        resource.id = %uid,
        )
    )]
    // Refuses to delete a connection type while connections still reference it,
    // which would leave those connections pointing at a type that no longer exists.
    //
    // Locking the type row before counting is what makes the check race-free.
    // create_data_connection and update_data_connection both take FOR SHARE on the
    // row of the type they are about to reference, in the same transaction as the
    // write, so FOR UPDATE here is mutually exclusive with them:
    //
    //   - a create that commits first is visible to the count below, which then
    //     refuses the delete;
    //   - a create that arrives while this transaction holds the lock blocks, and
    //     once this transaction commits its FOR SHARE matches no row, so it fails
    //     validation with "connection type not found" instead of writing an orphan.
    //
    // The count therefore has to run inside this transaction and after the lock.
    // It relies on READ COMMITTED (the default): each statement takes a fresh
    // snapshot, so the count sees connections committed while we waited on the lock.
    async fn delete_data_connection_type(&self, tenant_id: &str, uid: &str) -> Result<(), MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("failed to begin transaction: {e}");
            MetaStoreError::Query("failed to delete connection type".to_string())
        })?;

        sqlx::query(
            "SELECT 1 FROM data_connection_types \
             WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2 FOR UPDATE",
        )
        .bind(uid)
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => MetaStoreError::ResourceNotFound(format!("connection type '{uid}' not found")),
            e => {
                error!("failed to lock connection type '{uid}' for delete: {e}");
                MetaStoreError::Query("failed to delete connection type".to_string())
            },
        })?;

        // Deliberately not filtered by tenant. A global connection type is visible to
        // every tenant, so the connections referencing it may live in any of them;
        // scoping this to the caller's tenant would let a global type be deleted out
        // from under other tenants' connections.
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM data_connections WHERE data->'resource'->>'data_connection_type_id' = $1",
        )
        .bind(uid)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            error!("failed to count connections referencing connection type '{uid}': {e}");
            MetaStoreError::Query("failed to delete connection type".to_string())
        })?;

        if count > 0 {
            return Err(MetaStoreError::Conflict(connection_type_in_use_message(uid, count)));
        }

        let result = sqlx::query(
            "DELETE FROM data_connection_types WHERE data->'metadata'->>'id' = $1 AND data->'metadata'->>'tenant_id' = $2",
        )
        .bind(uid)
        .bind(tenant_id)
        .execute(&mut *tx)
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

        tx.commit().await.map_err(|e| {
            error!("failed to commit transaction: {e}");
            MetaStoreError::Query("failed to delete connection type".to_string())
        })?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl FlightDiscoveryStore for PgMetaStore {
    async fn create_flight_service(
        &self,
        flight_service: &FlightService,
    ) -> Result<FlightServiceResource, MetaStoreError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resource = FlightServiceResource {
            metadata: ResourceMetadata {
                id: Uuid::new_v4().to_string(),
                tenant_id: None,
                created_at: now.clone(),
                updated_at: now,
            },
            resource: flight_service.clone(),
        };

        let json_value = serde_json::to_value(&resource).map_err(|e| {
            error!("failed to serialize flight service: {e}");
            MetaStoreError::Serialization("failed to serialize flight service".to_string())
        })?;

        sqlx::query("INSERT INTO flight_services (data) VALUES ($1)")
            .bind(&json_value)
            .execute(&self.pool)
            .await
            .map_err(|e| match map_sqlx_error(e) {
                MetaStoreError::Conflict(_) => MetaStoreError::Conflict(format!(
                    "a flight service named '{}' already exists in namespace '{}'",
                    flight_service.name, flight_service.namespace
                )),
                other => other,
            })?;

        Ok(resource)
    }

    async fn get_all_flight_services(&self) -> Result<ResourceList<FlightServiceResource>, MetaStoreError> {
        let rows = sqlx::query("SELECT data FROM flight_services")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!("failed to list flight services: {e}");
                MetaStoreError::Query("failed to list flight services".to_string())
            })?;

        let items: Vec<FlightServiceResource> = rows
            .iter()
            .map(|row| {
                let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
                    error!("failed to read flight service column: {e}");
                    MetaStoreError::Query("failed to read flight service".to_string())
                })?;
                serde_json::from_value(json_value).map_err(|e| {
                    error!("failed to deserialize flight service: {e}");
                    MetaStoreError::Deserialization("failed to deserialize flight service".to_string())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ResourceList {
            total_count: items.len(),
            items,
        })
    }

    async fn get_flight_service_by_connector(&self, connector: &str) -> Result<FlightServiceResource, MetaStoreError> {
        let row =
            sqlx::query("SELECT data FROM flight_services WHERE data->'resource'->'supported_connectors' @> $1::jsonb")
                .bind(serde_json::json!([connector]))
                .fetch_one(&self.pool)
                .await
                .map_err(|e| match e {
                    sqlx::Error::RowNotFound => {
                        MetaStoreError::ResourceNotFound(format!("no flight service supports connector '{connector}'"))
                    },
                    e => {
                        error!("failed to find flight service for connector '{connector}': {e}");
                        MetaStoreError::Query("failed to retrieve flight service".to_string())
                    },
                })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read flight service column: {e}");
            MetaStoreError::Query("failed to read flight service".to_string())
        })?;
        serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize flight service: {e}");
            MetaStoreError::Deserialization("failed to deserialize flight service".to_string())
        })
    }

    async fn get_flight_service(&self, id: &str) -> Result<FlightServiceResource, MetaStoreError> {
        let row = sqlx::query("SELECT data FROM flight_services WHERE data->'metadata'->>'id' = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    MetaStoreError::ResourceNotFound(format!("flight service '{id}' not found"))
                },
                e => {
                    error!("failed to get flight service '{id}': {e}");
                    MetaStoreError::Query("failed to retrieve flight service".to_string())
                },
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read flight service column: {e}");
            MetaStoreError::Query("failed to read flight service".to_string())
        })?;
        serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize flight service: {e}");
            MetaStoreError::Deserialization("failed to deserialize flight service".to_string())
        })
    }

    async fn update_flight_service(
        &self,
        id: &str,
        update_fn: Arc<dyn Fn(FlightService) -> Result<FlightService, MetaStoreError> + Send + Sync>,
    ) -> Result<FlightServiceResource, MetaStoreError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("failed to begin transaction: {e}");
            MetaStoreError::Query("failed to update flight service".to_string())
        })?;

        let row = sqlx::query("SELECT data FROM flight_services WHERE data->'metadata'->>'id' = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    MetaStoreError::ResourceNotFound(format!("flight service '{id}' not found"))
                },
                e => {
                    error!("failed to get flight service '{id}' for update: {e}");
                    MetaStoreError::Query("failed to update flight service".to_string())
                },
            })?;

        let json_value: serde_json::Value = row.try_get("data").map_err(|e| {
            error!("failed to read flight service column: {e}");
            MetaStoreError::Query("failed to read flight service".to_string())
        })?;
        let mut existing: FlightServiceResource = serde_json::from_value(json_value).map_err(|e| {
            error!("failed to deserialize flight service: {e}");
            MetaStoreError::Deserialization("failed to deserialize flight service".to_string())
        })?;

        let id = existing.metadata.id.clone();
        let service = update_fn(existing.resource)?;

        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        existing.metadata.updated_at = now;
        existing.resource = service;

        let json_value = serde_json::to_value(&existing).map_err(|e| {
            error!("failed to serialize flight service: {e}");
            MetaStoreError::Serialization("failed to serialize flight service".to_string())
        })?;

        sqlx::query("UPDATE flight_services SET data = $1 WHERE data->'metadata'->>'id' = $2")
            .bind(&json_value)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!("failed to update flight service '{id}': {e}");
                MetaStoreError::Query("failed to update flight service".to_string())
            })?;

        tx.commit().await.map_err(|e| {
            error!("failed to commit transaction: {e}");
            MetaStoreError::Query("failed to update flight service".to_string())
        })?;

        Ok(existing)
    }

    async fn delete_flight_service(&self, id: &str) -> Result<(), MetaStoreError> {
        let result = sqlx::query("DELETE FROM flight_services WHERE data->'metadata'->>'id' = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("failed to delete flight service '{id}': {e}");
                MetaStoreError::Query("failed to delete flight service".to_string())
            })?;

        if result.rows_affected() == 0 {
            return Err(MetaStoreError::ResourceNotFound(format!(
                "flight service '{id}' not found"
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against a real PostgreSQL database. It rewrites the global tenant
    /// setting, so point it at a disposable database:
    ///
    /// ```text
    /// DCH_TEST_DATABASE_URL=postgresql://postgres:pw@localhost:5432/postgres \
    ///   cargo test -p pg-meta-store -- --ignored sync_global_tenant
    /// ```
    #[tokio::test]
    #[ignore = "requires DCH_TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
    async fn test_sync_global_tenant_migrates_connection_types() {
        let url = std::env::var("DCH_TEST_DATABASE_URL").expect("DCH_TEST_DATABASE_URL must be set");
        let suffix = Uuid::new_v4().simple().to_string();
        let old_tenant = format!("dch-old-{suffix}");
        let new_tenant = format!("dch-new-{suffix}");

        let old_store = PgMetaStore::new(DatabaseConfig { url: url.clone() }, old_tenant.clone())
            .await
            .unwrap();
        sqlx::query("DELETE FROM dch_settings WHERE key = $1")
            .bind(GLOBAL_TENANT_SETTING)
            .execute(&old_store.pool)
            .await
            .unwrap();

        assert_eq!(
            old_store.sync_global_tenant().await.unwrap(),
            GlobalTenantSync::Initialized
        );
        assert_eq!(
            old_store.sync_global_tenant().await.unwrap(),
            GlobalTenantSync::Unchanged
        );

        let insert = |id: String, tenant: String, name: &'static str| {
            let pool = old_store.pool.clone();
            async move {
                sqlx::query("INSERT INTO data_connection_types (data) VALUES ($1)")
                    .bind(serde_json::json!({
                        "metadata": {"id": id, "tenant_id": tenant},
                        "resource": {"name": name, "provider": "s3", "credentials_fields": []},
                    }))
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        };
        // S3 moves; URI conflicts with a type already in the new tenant.
        insert(format!("s3-{suffix}"), old_tenant.clone(), "S3").await;
        insert(format!("uri-old-{suffix}"), old_tenant.clone(), "URI").await;
        insert(format!("uri-new-{suffix}"), new_tenant.clone(), "URI").await;

        let new_store = PgMetaStore::new(DatabaseConfig { url }, new_tenant.clone())
            .await
            .unwrap();
        assert_eq!(
            new_store.sync_global_tenant().await.unwrap(),
            GlobalTenantSync::Migrated {
                previous: old_tenant.clone(),
                moved: 1,
                conflicts: 1,
            }
        );
        assert_eq!(
            new_store.sync_global_tenant().await.unwrap(),
            GlobalTenantSync::Unchanged
        );

        let tenant_of = |id: String| {
            let pool = new_store.pool.clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "SELECT data->'metadata'->>'tenant_id' FROM data_connection_types WHERE data->'metadata'->>'id' = $1",
                )
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(tenant_of(format!("s3-{suffix}")).await, new_tenant);
        assert_eq!(tenant_of(format!("uri-old-{suffix}")).await, old_tenant);
        assert_eq!(tenant_of(format!("uri-new-{suffix}")).await, new_tenant);

        sqlx::query("DELETE FROM data_connection_types WHERE data->'metadata'->>'tenant_id' IN ($1, $2)")
            .bind(&old_tenant)
            .bind(&new_tenant)
            .execute(&new_store.pool)
            .await
            .unwrap();
    }

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
        let msg = connection_type_in_use_message("ct-1", 1);
        assert!(msg.contains("ct-1"), "got: {msg}");
        assert!(msg.contains("1 connection still references it"), "got: {msg}");
        assert!(msg.contains("delete the connections first"), "got: {msg}");
    }

    #[test]
    fn test_connection_type_in_use_message_plural() {
        let msg = connection_type_in_use_message("ct-1", 3);
        assert!(msg.contains("3 connections still reference it"), "got: {msg}");
    }

    #[test]
    fn test_connection_type_in_use_message_discloses_no_connection_names() {
        // The count alone: a global connection type may be referenced from tenants
        // the caller cannot otherwise see.
        let msg = connection_type_in_use_message("ct-1", 2);
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
