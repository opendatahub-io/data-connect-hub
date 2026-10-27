use crate::flight::QueryContext;
use crate::flight::errors::map_connector_error;
use crate::flight::service::DataIngestionService;
use arrow::array::{Array, StringArray};
use arrow::record_batch::RecordBatch;
use arrow_flight::{Action, ActionType, flight_service_server::FlightService};
use commons::api::ResourceMetadata;
use commons::api::connections::CredentialsRef;
use commons::api::connections::DataConnection;
use commons::api::connections::DataConnectionResource;
use commons::api::connections::DataConnectionStatus;
use commons::api::connections::DataFormat;
use commons::api::connector::CredentialsResolver;
use commons::api::errors::ConnectorError;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::info;
use uuid::Uuid;

const ACTION_GET_SUPPORTED_CONNECTORS: &str = "GetSupportedConnectors";
const ACTION_CHECK_DATA_CONNECTION: &str = "CheckDataConnection";
const ACTION_CHECK_CREDENTIALS: &str = "CheckCredentials";

struct InlineCredsResolver {
    secret: HashMap<String, String>,
}

impl InlineCredsResolver {
    pub fn new(secret: HashMap<String, String>) -> Self {
        Self { secret }
    }
}

#[async_trait::async_trait]
impl CredentialsResolver for InlineCredsResolver {
    async fn resolve(&self, _connection: &DataConnectionResource) -> Result<HashMap<String, String>, ConnectorError> {
        Ok(self.secret.clone())
    }
}

impl DataIngestionService {
    pub fn custom_actions() -> Vec<Result<ActionType, Status>> {
        vec![
            Ok(ActionType {
                r#type: ACTION_GET_SUPPORTED_CONNECTORS.into(),
                description: "Returns the list of supported data connectors".into(),
            }),
            Ok(ActionType {
                r#type: ACTION_CHECK_DATA_CONNECTION.into(),
                description: "Checks the connection to the data source".into(),
            }),
            Ok(ActionType {
                r#type: ACTION_CHECK_CREDENTIALS.into(),
                description: "Checks the credentials without having to create a data connection".into(),
            }),
        ]
    }

    pub async fn dispatch_action(
        &self,
        request: Request<Action>,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let action = request.get_ref();
        match action.r#type.as_str() {
            ACTION_CHECK_DATA_CONNECTION => self.action_check_connection(&request).await,
            ACTION_GET_SUPPORTED_CONNECTORS => self.action_get_supported_connectors().await,
            ACTION_CHECK_CREDENTIALS => self.action_check_credentials(&request).await,
            _ => Err(Status::invalid_argument(format!("Unknown action: {}", action.r#type))),
        }
    }

    async fn action_check_credentials(
        &self,
        request: &Request<Action>,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let metadata = request.metadata();

        let tenant_id = QueryContext::tenant_id(metadata)?;

        let mut keys = parse_kv_body(&request.get_ref().body)?;

        let dct_id = keys
            .remove("data_connection_type_id")
            .ok_or(Status::invalid_argument("data_connection_type_id is required"))?;

        let credentials: HashMap<String, String> = keys
            .into_iter()
            .filter(|(k, _)| k.starts_with("secret."))
            .map(|(k, v)| (k.strip_prefix("secret.").unwrap().to_string(), v))
            .collect();

        let (data_connection_type, connector) = self.get_connector_by_type_id(tenant_id, &dct_id).await?;

        data_connection_type
            .resource
            .check_credentials_schema(&credentials.clone())
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resolver = InlineCredsResolver::new(credentials.clone());

        connector
            .get_reader(
                &DataConnectionResource {
                    metadata: ResourceMetadata {
                        id: Uuid::new_v4().to_string(),
                        tenant_id: Some(tenant_id.to_string()),
                        created_at: "2021-01-01".to_string(),
                        updated_at: "2021-01-01".to_string(),
                    },
                    resource: DataConnection {
                        name: "test".to_string(),
                        data_connection_type_id: dct_id.clone(),
                        format: DataFormat::Tabular,
                        credentials_ref: CredentialsRef::default(),
                        properties: HashMap::new(),
                    },
                    status: DataConnectionStatus::default(),
                },
                &resolver,
            )
            .await
            .map_err(map_connector_error)?;

        let result = arrow_flight::Result {
            body: Vec::new().into(),
        };
        Ok(Response::new(
            Box::pin(futures::stream::once(async { Ok(result) })) as <Self as FlightService>::DoActionStream
        ))
    }

    async fn action_check_connection(
        &self,
        request: &Request<Action>,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let metadata = request.metadata();

        let tenant_id = QueryContext::tenant_id(metadata)?;

        let connection_id = QueryContext::connection_id(metadata)?;
        let (connection, connector) = self.get_connector_by_connection_id(tenant_id, connection_id).await?;

        self.check_secret_access(metadata, tenant_id, &connection).await?;

        let reader = connector
            .get_reader(&connection, self as &dyn CredentialsResolver)
            .await
            .map_err(map_connector_error)?;

        reader.check_connection().await.map_err(map_connector_error)?;
        info!("Connection checked successfully");
        let result = arrow_flight::Result {
            body: Vec::new().into(),
        };
        Ok(Response::new(
            Box::pin(futures::stream::once(async { Ok(result) })) as <Self as FlightService>::DoActionStream
        ))
    }

    async fn action_get_supported_connectors(
        &self,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let connectors = self.connectors_registry.get_supported_connectors();
        let names: Vec<String> = connectors.iter().map(|c| c.provider()).collect();
        let descriptions: Vec<String> = connectors.iter().map(|c| c.description()).collect();
        // Comma-separated within a single Utf8 column; the REST client splits on ',' when parsing.
        let formats: Vec<String> = connectors
            .iter()
            .map(|c| {
                c.supported_formats()
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect();

        let batch = RecordBatch::try_from_iter(vec![
            ("name", Arc::new(StringArray::from(names)) as _),
            ("description", Arc::new(StringArray::from(descriptions)) as _),
            ("supported_formats", Arc::new(StringArray::from(formats)) as _),
        ])
        .map_err(|e| {
            tracing::error!(error = %e, "failed to build connector record batch");
            Status::internal("failed to build response")
        })?;

        let mut buf = Vec::new();
        {
            let mut writer = arrow::ipc::writer::StreamWriter::try_new(&mut buf, &batch.schema()).map_err(|e| {
                tracing::error!(error = %e, "failed to create IPC writer");
                Status::internal("failed to encode response")
            })?;
            writer.write(&batch).map_err(|e| {
                tracing::error!(error = %e, "failed to write IPC batch");
                Status::internal("failed to encode response")
            })?;
            writer.finish().map_err(|e| {
                tracing::error!(error = %e, "failed to finish IPC stream");
                Status::internal("failed to encode response")
            })?;
        }

        let result = arrow_flight::Result { body: buf.into() };
        Ok(Response::new(
            Box::pin(futures::stream::once(async { Ok(result) })) as <Self as FlightService>::DoActionStream
        ))
    }
}

/// Parses an Arrow IPC stream containing credentials from the action body.
///
/// Returns `None` if the body is empty. Otherwise expects a single `RecordBatch`
fn parse_kv_body(body: &[u8]) -> Result<HashMap<String, String>, Status> {
    let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(body), None)
        .map_err(|e| Status::invalid_argument(format!("invalid credentials payload: {e}")))?;

    let batch: RecordBatch = reader
        .into_iter()
        .next()
        .ok_or_else(|| Status::invalid_argument("credentials payload is empty"))?
        .map_err(|e| Status::invalid_argument(format!("failed to read credentials: {e}")))?;

    let keys = batch
        .column_by_name("key")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| Status::invalid_argument("credentials payload missing 'key' column"))?;

    let values = batch
        .column_by_name("value")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| Status::invalid_argument("credentials payload missing 'value' column"))?;

    let map: HashMap<String, String> = (0..keys.len())
        .map(|i| (keys.value(i).to_string(), values.value(i).to_string()))
        .collect();

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::registry::ConnectorsRegistry;
    use crate::flight::service::SecretAuthorizer;
    use commons::api::X_DATA_CONNECTION_ID;
    use commons::api::X_TENANT_ID;
    use commons::api::connector::{CredentialsResolver, DataReader, FlightConnector, QueryOptions};
    use commons::api::storage::{FlightDiscoveryStore, MetaStore, MetaStoreReader, SecretStore};
    use commons::api::{ResourceList, secret::Secret};
    use kube_utils::auth::{AuthError, AuthInfo};
    use std::sync::Arc;

    struct StubConnector;

    #[async_trait::async_trait]
    impl FlightConnector for StubConnector {
        fn provider(&self) -> String {
            "postgres".to_string()
        }

        fn description(&self) -> String {
            "stub postgres connector".to_string()
        }

        async fn get_reader(
            &self,
            _data_connection: &DataConnectionResource,
            _credentials_resolver: &dyn CredentialsResolver,
        ) -> Result<Arc<dyn DataReader>, ConnectorError> {
            Err(ConnectorError::ConfigError(
                "reader must not be reached when access is denied".to_string(),
            ))
        }
    }

    fn connection() -> DataConnectionResource {
        DataConnectionResource {
            metadata: ResourceMetadata {
                id: "conn-1".to_string(),
                tenant_id: Some("tenant-a".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: DataConnection {
                name: "my-pg".to_string(),
                data_connection_type_id: "ct-1".to_string(),
                format: DataFormat::Tabular,
                credentials_ref: CredentialsRef {
                    secret: "db-secret".to_string(),
                },
                properties: HashMap::new(),
            },
            status: Default::default(),
        }
    }

    fn connection_type() -> commons::api::connection_types::DataConnectionTypeResource {
        commons::api::connection_types::DataConnectionTypeResource {
            metadata: ResourceMetadata {
                id: "ct-1".to_string(),
                tenant_id: Some("tenant-a".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: commons::api::connection_types::DataConnectionType {
                name: "PostgreSQL".to_string(),
                label: None,
                provider: "postgres".to_string(),
                description: None,
                credentials_fields: vec![],
                tags: None,
            },
            status: Default::default(),
        }
    }

    struct StubMetaStore;

    #[async_trait::async_trait]
    impl MetaStoreReader for StubMetaStore {
        async fn get_data_connections(
            &self,
            _tenant_id: &str,
        ) -> Result<ResourceList<DataConnectionResource>, commons::api::errors::MetaStoreError> {
            Err(commons::api::errors::MetaStoreError::ResourceNotFound(
                "unused".to_string(),
            ))
        }

        async fn get_data_connection(
            &self,
            tenant_id: &str,
            uid: &str,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            if tenant_id == "tenant-a" && uid == "conn-1" {
                Ok(connection())
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "connection '{uid}' not found"
                )))
            }
        }

        async fn get_data_connection_type(
            &self,
            tenant_id: &str,
            id: &str,
        ) -> Result<commons::api::connection_types::DataConnectionTypeResource, commons::api::errors::MetaStoreError>
        {
            if tenant_id == "tenant-a" && id == "ct-1" {
                Ok(connection_type())
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "connection type '{id}' not found"
                )))
            }
        }

        async fn get_data_connection_types(
            &self,
            _tenant_id: &str,
        ) -> Result<
            ResourceList<commons::api::connection_types::DataConnectionTypeResource>,
            commons::api::errors::MetaStoreError,
        > {
            Err(commons::api::errors::MetaStoreError::ResourceNotFound(
                "unused".to_string(),
            ))
        }
    }

    #[async_trait::async_trait]
    impl FlightDiscoveryStore for StubMetaStore {
        async fn create_flight_service(
            &self,
            _: &commons::api::flight_discovery::FlightService,
        ) -> Result<commons::api::flight_discovery::FlightServiceResource, commons::api::errors::MetaStoreError>
        {
            unimplemented!()
        }

        async fn get_all_flight_services(
            &self,
        ) -> Result<
            ResourceList<commons::api::flight_discovery::FlightServiceResource>,
            commons::api::errors::MetaStoreError,
        > {
            unimplemented!()
        }

        async fn get_flight_service_by_connector(
            &self,
            _: &str,
        ) -> Result<commons::api::flight_discovery::FlightServiceResource, commons::api::errors::MetaStoreError>
        {
            unimplemented!()
        }

        async fn get_flight_service(
            &self,
            _: &str,
        ) -> Result<commons::api::flight_discovery::FlightServiceResource, commons::api::errors::MetaStoreError>
        {
            unimplemented!()
        }

        async fn update_flight_service(
            &self,
            _: &str,
            _: std::sync::Arc<
                dyn Fn(
                        commons::api::flight_discovery::FlightService,
                    )
                        -> Result<commons::api::flight_discovery::FlightService, commons::api::errors::MetaStoreError>
                    + Send
                    + Sync,
            >,
        ) -> Result<commons::api::flight_discovery::FlightServiceResource, commons::api::errors::MetaStoreError>
        {
            unimplemented!()
        }

        async fn delete_flight_service(&self, _: &str) -> Result<(), commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
    }

    #[async_trait::async_trait]
    impl MetaStore for StubMetaStore {
        async fn create_data_connection(
            &self,
            _tenant_id: &str,
            _data_connection: &DataConnection,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }

        async fn update_data_connection(
            &self,
            _tenant_id: &str,
            _uid: &str,
            _update_fn: std::sync::Arc<
                dyn Fn(DataConnection) -> Result<DataConnection, commons::api::errors::MetaStoreError> + Send + Sync,
            >,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }

        async fn update_data_connection_status(
            &self,
            _tenant_id: &str,
            _uid: &str,
            _update_fn: std::sync::Arc<
                dyn Fn(DataConnectionStatus) -> Result<DataConnectionStatus, commons::api::errors::MetaStoreError>
                    + Send
                    + Sync,
            >,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }

        async fn delete_data_connection(
            &self,
            _tenant_id: &str,
            _uid: &str,
        ) -> Result<(), commons::api::errors::MetaStoreError> {
            unimplemented!()
        }

        async fn create_data_connection_type(
            &self,
            _tenant_id: &str,
            _data_connection_type: &commons::api::connection_types::DataConnectionType,
        ) -> Result<commons::api::connection_types::DataConnectionTypeResource, commons::api::errors::MetaStoreError>
        {
            unimplemented!()
        }

        async fn update_data_connection_type(
            &self,
            _tenant_id: &str,
            _uid: &str,
            _update_fn: std::sync::Arc<
                dyn Fn(
                        commons::api::connection_types::DataConnectionType,
                    ) -> Result<
                        commons::api::connection_types::DataConnectionType,
                        commons::api::errors::MetaStoreError,
                    > + Send
                    + Sync,
            >,
        ) -> Result<commons::api::connection_types::DataConnectionTypeResource, commons::api::errors::MetaStoreError>
        {
            unimplemented!()
        }

        async fn update_data_connection_type_status(
            &self,
            _uid: &str,
            _update_fn: std::sync::Arc<
                dyn Fn(
                        commons::api::connection_types::DataConnectionTypeStatus,
                    ) -> Result<
                        commons::api::connection_types::DataConnectionTypeStatus,
                        commons::api::errors::MetaStoreError,
                    > + Send
                    + Sync,
            >,
        ) -> Result<commons::api::connection_types::DataConnectionTypeResource, commons::api::errors::MetaStoreError>
        {
            unimplemented!()
        }

        async fn delete_data_connection_type(
            &self,
            _tenant_id: &str,
            _uid: &str,
        ) -> Result<(), commons::api::errors::MetaStoreError> {
            unimplemented!()
        }

        async fn get_all_data_connection_types(
            &self,
        ) -> Result<
            ResourceList<commons::api::connection_types::DataConnectionTypeResource>,
            commons::api::errors::MetaStoreError,
        > {
            unimplemented!()
        }
    }

    struct StubSecretStore;

    #[async_trait::async_trait]
    impl SecretStore for StubSecretStore {
        async fn get_secret(
            &self,
            _namespace: &str,
            _name: &str,
        ) -> Result<Secret, commons::api::errors::SecretStoreError> {
            Err(commons::api::errors::SecretStoreError::SecretNotFound(
                "unused".to_string(),
            ))
        }

        async fn create_secret(
            &self,
            _secret: &Secret,
            _overwrite: bool,
        ) -> Result<(), commons::api::errors::SecretStoreError> {
            Err(commons::api::errors::SecretStoreError::CannotCreateSecret(
                "unused".to_string(),
            ))
        }

        async fn delete_secret(
            &self,
            _namespace: &str,
            _name: &str,
        ) -> Result<(), commons::api::errors::SecretStoreError> {
            Err(commons::api::errors::SecretStoreError::SecretNotFound(
                "unused".to_string(),
            ))
        }

        async fn set_secret_labels(
            &self,
            _namespace: &str,
            _name: &str,
            _labels: HashMap<String, String>,
        ) -> Result<(), commons::api::errors::SecretStoreError> {
            Err(commons::api::errors::SecretStoreError::SecretNotFound(
                "unused".to_string(),
            ))
        }
    }

    struct DenyAllAuthorizer;

    #[async_trait::async_trait]
    impl SecretAuthorizer for DenyAllAuthorizer {
        async fn authorize_secret(
            &self,
            _auth_info: &AuthInfo,
            _tenant_id: &str,
            _secret_name: &str,
        ) -> Result<(), AuthError> {
            Err(AuthError::Forbidden("stub denial".to_string()))
        }
    }

    fn service_with_authorizer() -> DataIngestionService {
        let registry = ConnectorsRegistry::new().with_connector(Arc::new(StubConnector));
        DataIngestionService::new(
            Arc::new(registry),
            Arc::new(StubMetaStore),
            Arc::new(StubSecretStore),
            QueryOptions::default(),
        )
        .with_authorizer(Arc::new(DenyAllAuthorizer))
    }

    fn check_connection_request() -> Request<Action> {
        let mut request = Request::new(Action {
            r#type: ACTION_CHECK_DATA_CONNECTION.to_string(),
            body: Vec::new().into(),
        });
        let metadata = request.metadata_mut();
        metadata.insert(X_TENANT_ID, "tenant-a".parse().unwrap());
        metadata.insert(X_DATA_CONNECTION_ID, "conn-1".parse().unwrap());
        metadata.insert(
            "x-remote-user",
            "system:serviceaccount:tenant-a:dch-test-user".parse().unwrap(),
        );
        request
    }

    #[tokio::test]
    async fn check_data_connection_denies_user_without_secret_access() {
        let service = service_with_authorizer();

        let err = service
            .dispatch_action(check_connection_request())
            .await
            .err()
            .expect("action must be denied");

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }
}
