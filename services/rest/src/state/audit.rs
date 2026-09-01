use commons::api::connection_types::DataConnectionTypeStatus;
use commons::api::connections::CredentialsRef;

use commons::api::connections::DataConnectionResource;
use commons::api::storage::MetaStore;

use crate::clients::flight::FlightClient;
use crate::rest::errors::ValidationError;
use chrono::Utc;
use commons::api::connection_types::DataConnectionTypeResource;
use commons::api::connections::DataConnectionState;
use commons::api::connections::DataConnectionStatus;
use commons::api::connections::DataFormat;
use commons::api::storage::SecretStore;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tracing::info;

async fn set_data_connection_status(
    tenant_id: &str,
    data_connection_id: &str,
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    status: DataConnectionState,
    message: Option<String>,
) -> Result<(), ValidationError> {
    let update_fn = Arc::new(move |_: DataConnectionStatus| {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        Ok(DataConnectionStatus {
            state: status.clone(),
            message: message.clone(),
            updated_at: Some(now),
        })
    });
    meta_store
        .update_data_connection_status(tenant_id, data_connection_id, update_fn)
        .await
        .map_err(|_| ValidationError::StatusUpdateFailed("Failed to update data connection status".to_string()))?;
    Ok(())
}

pub async fn audit_data_connection(
    tenant_id: &str,
    data_connection_id: &str,
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    flight_client: &FlightClient,
) -> Result<(), ValidationError> {
    let data_connection = meta_store
        .get_data_connection(tenant_id, data_connection_id)
        .await
        .map_err(|e| ValidationError::ConnectionCheckFailed(data_connection_id.to_string()))?;

    let dct = meta_store
        .get_data_connection_type(tenant_id, &data_connection.resource.data_connection_type_id)
        .await
        .map_err(|_| ValidationError::InvalidDataConnectionType)?;

    let keys = {
        let secret = secret_store
            .get_secret(tenant_id, data_connection.resource.credentials_ref.secret.as_str())
            .await
            .map_err(|_| ValidationError::InvalidSecret);
        if let Ok(secret) = secret {
            secret.properties
        } else {
            set_data_connection_status(
                tenant_id,
                data_connection_id,
                meta_store,
                DataConnectionState::NotReady,
                Some("Secret cannot be read".to_string()),
            )
            .await?;
            return Err(ValidationError::InvalidSecret);
        }
    };

    let result = dct.resource.check_credentials_schema(&keys);

    if let Err(e) = result {
        set_data_connection_status(
            tenant_id,
            data_connection_id,
            meta_store,
            DataConnectionState::NotReady,
            Some(e.to_string()),
        )
        .await?;
        return Err(ValidationError::CredentialsCheckFailed(e.to_string()));
    }

    let connection_id = data_connection.metadata.id.clone();
    let result = flight_client.check_data_connection(tenant_id, &connection_id).await;

    match result {
        Ok(_) => {
            set_data_connection_status(
                tenant_id,
                data_connection_id,
                meta_store,
                DataConnectionState::Ready,
                Some("Connection check successful".to_string()),
            )
            .await?;
        },
        Err(_) => {
            set_data_connection_status(
                tenant_id,
                data_connection_id,
                meta_store,
                DataConnectionState::IngestionNotReady,
                Some("Connection check failed".to_string()),
            )
            .await?;

            return Err(ValidationError::ConnectionCheckFailed(connection_id));
        },
    };
    Ok(())
}

pub async fn audit_data_connection_types(
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    flight_client: &FlightClient,
) -> Result<(), ValidationError> {
    let supported = flight_client.get_supported_connectors().await.map_err(|e| {
        tracing::error!(error = %e, "failed to get supported connectors from flight service");
        ValidationError::FlightServiceError(e.to_string())
    })?;

    let supported_names: Vec<&str> = supported.iter().map(|c| c.name.as_str()).collect();

    info!("supported connectors: {:?}", supported_names.join(", "));

    let data_connection_types = meta_store
        .get_all_data_connection_types()
        .await
        .map_err(|_| ValidationError::InvalidDataConnectionType)?;

    for dct in &data_connection_types.items {
        info!(
            "Checking data connection type: {} {:?}",
            dct.resource.name, dct.resource.provider
        );

        let mut capabilities = dct.status.capabilities.clone();
        let flight = supported_names.contains(&dct.resource.provider.as_str());
        if capabilities.flight != flight {
            capabilities.flight = flight;

            info!("Capabilities after update: {:?}", capabilities);

            let update_fn = Arc::new(move |current: DataConnectionTypeStatus| {
                let mut status = current.capabilities.clone();
                status.flight = capabilities.flight;
                Ok(DataConnectionTypeStatus { capabilities: status })
            });
            meta_store
                .update_data_connection_type_status(&dct.metadata.id, update_fn)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, provider = %dct.resource.provider, "failed to update connection type status");
                    ValidationError::InvalidDataConnectionType
                })?;
            info!("updated data connection type status: {:?}", dct.status);
        }
    }

    Ok(())
}

pub(crate) async fn audit_connection_type(
    flight_client: &FlightClient,
    meta_store: &Arc<dyn MetaStore + Send + Sync>,
    connection_type: DataConnectionTypeResource,
) -> Result<(), ValidationError> {
    let connectors = flight_client.get_supported_connectors().await;

    if let Ok(connectors) = connectors {
        let names: Vec<String> = connectors.into_iter().map(|c| c.name).collect();
        let provider = &connection_type.resource.provider;

        let supports_flight = Arc::new(AtomicBool::new(names.iter().any(|n| n == provider)));

        let update_fn = Arc::new(move |current: DataConnectionTypeStatus| {
            let mut status = current.capabilities.clone();
            status.flight = supports_flight.load(Ordering::Relaxed);

            Ok(DataConnectionTypeStatus { capabilities: status })
        });

        meta_store
            .update_data_connection_type_status(connection_type.metadata.id.as_str(), update_fn)
            .await
            .map_err(|e| ValidationError::StatusUpdateFailed(connection_type.metadata.id.clone()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use commons::api::ResourceList;
    use commons::api::ResourceMetadata;
    use commons::api::connection_types::{
        DataConnectionType, DataConnectionTypeResource, DataConnectionTypeStatus, Field,
    };
    use commons::api::connections::{DataConnection, DataConnectionState, DataConnectionStatus, DataFormat};
    use commons::api::errors::{MetaStoreError, SecretStoreError};
    use commons::api::secret::Secret;
    use std::collections::HashMap;
    use std::sync::RwLock;

    struct MockMetaStore {
        connection: Option<DataConnectionResource>,
        connection_type: Option<DataConnectionTypeResource>,
        last_status: RwLock<Option<DataConnectionStatus>>,
    }

    impl MockMetaStore {
        fn with_connection_and_type(conn: DataConnectionResource, dct: DataConnectionTypeResource) -> Self {
            Self {
                connection: Some(conn),
                connection_type: Some(dct),
                last_status: RwLock::new(None),
            }
        }

        fn not_found() -> Self {
            Self {
                connection: None,
                connection_type: None,
                last_status: RwLock::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl MetaStore for MockMetaStore {
        async fn get_data_connections(&self, _: &str) -> Result<ResourceList<DataConnectionResource>, MetaStoreError> {
            unimplemented!()
        }
        async fn get_data_connection(&self, _: &str, _: &str) -> Result<DataConnectionResource, MetaStoreError> {
            self.connection
                .clone()
                .ok_or_else(|| MetaStoreError::ResourceNotFound("not found".into()))
        }
        async fn create_data_connection(
            &self,
            _: &str,
            _: &DataConnection,
        ) -> Result<DataConnectionResource, MetaStoreError> {
            unimplemented!()
        }
        async fn update_data_connection(
            &self,
            _: &str,
            _: &str,
            _: Arc<dyn Fn(DataConnection) -> Result<DataConnection, MetaStoreError> + Send + Sync>,
        ) -> Result<DataConnectionResource, MetaStoreError> {
            unimplemented!()
        }
        async fn update_data_connection_status(
            &self,
            _tenant_id: &str,
            _: &str,
            update_fn: Arc<dyn Fn(DataConnectionStatus) -> Result<DataConnectionStatus, MetaStoreError> + Send + Sync>,
        ) -> Result<DataConnectionResource, MetaStoreError> {
            let mut conn = self.connection.clone().unwrap();
            let new_status = update_fn(conn.status.clone())?;
            *self.last_status.write().unwrap() = Some(new_status.clone());
            conn.status = new_status;
            Ok(conn)
        }
        async fn delete_data_connection(&self, _: &str, _: &str) -> Result<(), MetaStoreError> {
            unimplemented!()
        }
        async fn get_data_connection_types(
            &self,
            _: &str,
        ) -> Result<ResourceList<DataConnectionTypeResource>, MetaStoreError> {
            unimplemented!()
        }
        async fn get_all_data_connection_types(
            &self,
        ) -> Result<ResourceList<DataConnectionTypeResource>, MetaStoreError> {
            unimplemented!()
        }
        async fn get_data_connection_type(
            &self,
            _: &str,
            _: &str,
        ) -> Result<DataConnectionTypeResource, MetaStoreError> {
            self.connection_type
                .clone()
                .ok_or_else(|| MetaStoreError::ResourceNotFound("not found".into()))
        }
        async fn create_data_connection_type(
            &self,
            _: &str,
            _: &DataConnectionType,
        ) -> Result<DataConnectionTypeResource, MetaStoreError> {
            unimplemented!()
        }
        async fn update_data_connection_type(
            &self,
            _: &str,
            _: &str,
            _: Arc<dyn Fn(DataConnectionType) -> Result<DataConnectionType, MetaStoreError> + Send + Sync>,
        ) -> Result<DataConnectionTypeResource, MetaStoreError> {
            unimplemented!()
        }
        async fn update_data_connection_type_status(
            &self,
            _: &str,
            _: Arc<dyn Fn(DataConnectionTypeStatus) -> Result<DataConnectionTypeStatus, MetaStoreError> + Send + Sync>,
        ) -> Result<DataConnectionTypeResource, MetaStoreError> {
            unimplemented!()
        }
        async fn delete_data_connection_type(&self, _: &str, _: &str) -> Result<(), MetaStoreError> {
            unimplemented!()
        }
    }

    struct MockSecretStore {
        secret: Option<Secret>,
    }

    #[async_trait::async_trait]
    impl SecretStore for MockSecretStore {
        async fn get_secret(&self, _: &str, _: &str) -> Result<Secret, SecretStoreError> {
            self.secret
                .clone()
                .ok_or_else(|| SecretStoreError::SecretNotFound("not found".into()))
        }
        async fn create_secret(&self, _: &Secret, _: bool) -> Result<(), SecretStoreError> {
            unimplemented!()
        }
        async fn delete_secret(&self, _: &str, _: &str) -> Result<(), SecretStoreError> {
            unimplemented!()
        }
        async fn set_secret_labels(
            &self,
            _: &str,
            _: &str,
            _: HashMap<String, String>,
        ) -> Result<(), SecretStoreError> {
            unimplemented!()
        }
    }

    fn make_metadata(id: &str) -> ResourceMetadata {
        ResourceMetadata {
            id: id.to_string(),
            tenant_id: Some("tenant".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_connection(creds: CredentialsRef) -> DataConnectionResource {
        DataConnectionResource {
            metadata: make_metadata("conn-1"),
            resource: DataConnection {
                name: "test".to_string(),
                data_connection_type_id: "pg".to_string(),
                format: DataFormat::Tabular,
                credentials_ref: creds,
                properties: HashMap::new(),
            },
            status: DataConnectionStatus {
                state: DataConnectionState::NotReady,
                message: None,
                updated_at: None,
            },
        }
    }

    fn make_dct(required_fields: Vec<&str>) -> DataConnectionTypeResource {
        DataConnectionTypeResource {
            metadata: make_metadata("pg"),
            resource: DataConnectionType {
                name: "PostgreSQL".to_string(),
                provider: "postgres".to_string(),
                description: None,
                credentials_fields: required_fields
                    .into_iter()
                    .map(|name| Field {
                        name: name.to_string(),
                        label: name.to_string(),
                        d_type: "string".to_string(),
                        description: None,
                        required: true,
                        enum_values: None,
                        default_value: None,
                    })
                    .collect(),
            },
            status: Default::default(),
        }
    }

    fn make_secret(keys: Vec<(&str, &str)>) -> Secret {
        Secret {
            name: "creds".to_string(),
            namespace: "tenant".to_string(),
            properties: keys.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            labels: None,
            annotations: None,
        }
    }

    fn flight_client() -> FlightClient {
        FlightClient::new("http://127.0.0.1:1".to_string())
    }

    #[tokio::test]
    async fn test_connection_not_found() {
        let meta = Arc::new(MockMetaStore::not_found()) as Arc<dyn MetaStore + Send + Sync>;
        let secrets = Arc::new(MockSecretStore { secret: None }) as Arc<dyn SecretStore + Send + Sync>;

        let result = audit_data_connection("tenant", "missing", meta, secrets, &flight_client()).await;
        assert!(matches!(result, Err(ValidationError::ConnectionCheckFailed(_))));
    }

    #[tokio::test]
    async fn test_secret_not_found() {
        let conn = make_connection(CredentialsRef {
            secret: "missing-secret".to_string(),
        });
        let dct = make_dct(vec!["HOST"]);
        let meta = Arc::new(MockMetaStore::with_connection_and_type(conn, dct));
        let secrets = Arc::new(MockSecretStore { secret: None });

        let result = audit_data_connection("tenant", "conn-1", meta, secrets, &flight_client()).await;
        assert!(matches!(result, Err(ValidationError::InvalidSecret)));
    }

    #[tokio::test]
    async fn test_credentials_schema_check_fails() {
        let conn = make_connection(CredentialsRef {
            secret: "creds".to_string(),
        });
        let dct = make_dct(vec!["HOST", "PORT"]);
        let meta = Arc::new(MockMetaStore::with_connection_and_type(conn, dct));
        let secrets = Arc::new(MockSecretStore {
            secret: Some(make_secret(vec![("HOST", "localhost")])),
        });

        let result = audit_data_connection("tenant", "conn-1", meta, secrets, &flight_client()).await;
        assert!(matches!(result, Err(ValidationError::CredentialsCheckFailed(_))));
    }

    #[tokio::test]
    async fn test_flight_check_fails() {
        let conn = make_connection(CredentialsRef {
            secret: "creds".to_string(),
        });
        let dct = make_dct(vec!["HOST"]);
        let meta = Arc::new(MockMetaStore::with_connection_and_type(conn, dct));
        let secrets = Arc::new(MockSecretStore {
            secret: Some(make_secret(vec![("HOST", "localhost")])),
        });

        let result = audit_data_connection("tenant", "conn-1", meta, secrets, &flight_client()).await;
        assert!(matches!(result, Err(ValidationError::ConnectionCheckFailed(_))));
    }

    #[tokio::test]
    async fn test_secret_not_found_sets_not_ready_status() {
        let conn = make_connection(CredentialsRef {
            secret: "missing-secret".to_string(),
        });
        let dct = make_dct(vec!["HOST"]);
        let meta = Arc::new(MockMetaStore::with_connection_and_type(conn, dct));
        let secrets = Arc::new(MockSecretStore { secret: None });

        let _ = audit_data_connection("tenant", "conn-1", meta.clone(), secrets, &flight_client()).await;

        let status = meta.last_status.read().unwrap();
        let status = status.as_ref().expect("status should have been updated");
        assert_eq!(status.state, DataConnectionState::NotReady);
        assert_eq!(status.message.as_deref(), Some("Secret cannot be read"));
    }

    #[tokio::test]
    async fn test_credentials_check_fails_sets_not_ready_status() {
        let conn = make_connection(CredentialsRef {
            secret: "creds".to_string(),
        });
        let dct = make_dct(vec!["HOST", "PORT"]);
        let meta = Arc::new(MockMetaStore::with_connection_and_type(conn, dct));
        let secrets = Arc::new(MockSecretStore {
            secret: Some(make_secret(vec![("HOST", "localhost")])),
        });

        let _ = audit_data_connection("tenant", "conn-1", meta.clone(), secrets, &flight_client()).await;

        let status = meta.last_status.read().unwrap();
        let status = status.as_ref().expect("status should have been updated");
        assert_eq!(status.state, DataConnectionState::NotReady);
        assert!(status.message.as_ref().unwrap().contains("PORT"));
    }

    #[tokio::test]
    async fn test_flight_check_fails_sets_ingestion_not_ready_status() {
        let conn = make_connection(CredentialsRef {
            secret: "creds".to_string(),
        });
        let dct = make_dct(vec!["HOST"]);
        let meta = Arc::new(MockMetaStore::with_connection_and_type(conn, dct));
        let secrets = Arc::new(MockSecretStore {
            secret: Some(make_secret(vec![("HOST", "localhost")])),
        });

        let _ = audit_data_connection("tenant", "conn-1", meta.clone(), secrets, &flight_client()).await;

        let status = meta.last_status.read().unwrap();
        let status = status.as_ref().expect("status should have been updated");
        assert_eq!(status.state, DataConnectionState::IngestionNotReady);
        assert_eq!(status.message.as_deref(), Some("Connection check failed"));
    }
}
