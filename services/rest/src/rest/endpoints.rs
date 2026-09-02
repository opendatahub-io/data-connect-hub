use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use super::errors::ValidationError;

use crate::clients::flight::FlightClient;
use crate::state::audit::audit_connection_type;
use crate::state::audit::audit_data_connection;
use crate::state::audit::audit_data_connection_types;
use crate::utils::default_secret_labels;

use actix_web::{HttpResponse, web};
use commons::api::connection_types::DataConnectionType;
use commons::api::connections::DataConnection;
use commons::api::creds::TestCredentials;
use commons::api::secret::Secret;
use commons::api::storage::MetaStore;
use commons::api::storage::SecretStore;
use serde::Serialize;

use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;
use tracing::info;

use crate::rest::CreateConnectionRequest;
use crate::rest::DataConnectionWithCreds;

#[derive(Clone)]
pub struct ApiContext {
    pub tenant_id: String,
}

#[derive(Serialize)]
struct HealthResponse {
    service: String,
}

pub struct ApiService {
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    flight_client: FlightClient,
}

impl ApiService {
    pub fn new(
        meta_store: Arc<dyn MetaStore + Send + Sync>,
        secret_store: Arc<dyn SecretStore + Send + Sync>,
        flight_client: FlightClient,
    ) -> Self {
        Self {
            meta_store,
            secret_store,
            flight_client,
        }
    }
}

pub async fn health() -> Result<HttpResponse, RestErrorResponse> {
    Ok(HttpResponse::Ok().json(HealthResponse {
        service: "Data Connect Hub".to_string(),
    }))
}

pub async fn list_connections(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("list_connections: for tenant {:?}", ctx.tenant_id);
    let connections = service.meta_store.get_data_connections(ctx.tenant_id.as_str()).await?;
    Ok(HttpResponse::Ok().json(connections))
}

pub async fn get_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("get_connection");
    let connection = service
        .meta_store
        .get_data_connection(ctx.tenant_id.as_str(), id.as_str())
        .await?;
    Ok(HttpResponse::Ok().json(connection))
}

async fn create_connection_with_creds(
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    tenant_id: String,
    dc_creds: DataConnectionWithCreds,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("create_connection_with_creds: for tenant {:?}", tenant_id);

    let dct = meta_store
        .get_data_connection_type(&tenant_id, &dc_creds.data_connection_type_id)
        .await?;

    dct.resource
        .check_credentials_schema(&dc_creds.credentials.properties)
        .map_err(|e| ValidationError::CredentialsCheckFailed(e.to_string()))?;

    let data_connection = dc_creds.to_data_connection();

    let secret_obj = Secret {
        name: dc_creds.credentials.secret.clone(),
        namespace: tenant_id.to_string(),
        properties: dc_creds.credentials.properties.clone(),
        labels: Some(default_secret_labels()),
        annotations: Some(HashMap::new()),
    };

    secret_store.create_secret(&secret_obj, false).await?;

    let connection_res = meta_store.create_data_connection(&tenant_id, &data_connection).await;

    match connection_res {
        Ok(connection_res) => Ok(HttpResponse::Created().json(connection_res)),
        Err(e) => {
            let res = secret_store.delete_secret(&tenant_id, &secret_obj.name).await;
            if let Err(e) = res {
                error!("Failed to delete secret: {:?}", e);
            }
            Err(e.into())
        },
    }
}

pub async fn create_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    body: web::Json<CreateConnectionRequest>,
) -> Result<HttpResponse, RestErrorResponse> {
    let meta_store = service.meta_store.clone();
    let secret_store = service.secret_store.clone();
    let tenant_id = ctx.tenant_id.clone();

    match body.into_inner() {
        CreateConnectionRequest::DataConnectionWithInlineCreds(dc_creds) => {
            create_connection_with_creds(meta_store, secret_store, tenant_id, dc_creds).await
        },
        CreateConnectionRequest::DataConnectionWithSecretRef(connection) => {
            info!("create_connection: for tenant {:?}", tenant_id);

            let connection_res = meta_store.create_data_connection(&tenant_id, &connection).await?;

            Ok(HttpResponse::Created().json(connection_res))
        },
    }
}

pub async fn list_connection_types(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("list_connection_types: for tenant {:?}", ctx.tenant_id);
    let connection_types = service
        .meta_store
        .get_data_connection_types(ctx.tenant_id.as_str())
        .await?;

    Ok(HttpResponse::Ok().json(connection_types))
}

pub async fn get_connection_type(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("get_connection_type: for tenant {:?}", ctx.tenant_id);
    let connection_type = service
        .meta_store
        .get_data_connection_type(ctx.tenant_id.as_str(), id.as_str())
        .await?;
    Ok(HttpResponse::Ok().json(connection_type))
}

pub async fn patch_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("patch_connection: for tenant {:?}", ctx.tenant_id);
    let id = id.into_inner();
    let patch = body.into_inner();

    let update_fn = Arc::new(move |conn: DataConnection| {
        let mut value = serde_json::to_value(&conn)
            .map_err(|e| commons::api::errors::MetaStoreError::Serialization(e.to_string()))?;
        json_patch::merge(&mut value, &patch);
        serde_json::from_value(value).map_err(|e| commons::api::errors::MetaStoreError::Deserialization(e.to_string()))
    });

    let connection = service
        .meta_store
        .update_data_connection(ctx.tenant_id.as_str(), id.as_str(), update_fn)
        .await?;

    Ok(HttpResponse::Ok().json(connection))
}

pub async fn create_connection_type(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    connection_type: web::Json<DataConnectionType>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("create_connection_type: for tenant {:?}", ctx.tenant_id);

    let connection_type = service
        .meta_store
        .create_data_connection_type(ctx.tenant_id.as_str(), &connection_type)
        .await?;

    audit_connection_type(&service.flight_client, &service.meta_store, connection_type.clone()).await?;

    Ok(HttpResponse::Created().json(connection_type))
}

pub async fn patch_connection_type(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("patch_connection_type: for tenant {:?}", ctx.tenant_id);
    let id = id.into_inner();
    let patch = body.into_inner();

    let update_fn = Arc::new(move |ct: DataConnectionType| {
        let mut value = serde_json::to_value(&ct)
            .map_err(|e| commons::api::errors::MetaStoreError::Serialization(e.to_string()))?;
        json_patch::merge(&mut value, &patch);
        serde_json::from_value(value).map_err(|e| commons::api::errors::MetaStoreError::Deserialization(e.to_string()))
    });

    let connection_type = service
        .meta_store
        .update_data_connection_type(ctx.tenant_id.as_str(), id.as_str(), update_fn)
        .await?;

    audit_connection_type(&service.flight_client, &service.meta_store, connection_type.clone()).await?;

    Ok(HttpResponse::Ok().json(connection_type))
}

pub async fn delete_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("delete_connection: for tenant {:?}", ctx.tenant_id);
    service
        .meta_store
        .delete_data_connection(ctx.tenant_id.as_str(), id.as_str())
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

pub async fn delete_connection_type(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("delete_connection_type: for tenant {:?}", ctx.tenant_id);
    service
        .meta_store
        .delete_data_connection_type(ctx.tenant_id.as_str(), id.as_str())
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

pub async fn get_ingestion_data(
    _service: web::Data<ApiService>,
    _ctx: web::ReqData<ApiContext>,
    _id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn audit_connection_types(service: web::Data<ApiService>) -> Result<HttpResponse, RestErrorResponse> {
    info!("audit_connection_types");
    audit_data_connection_types(service.meta_store.clone(), &service.flight_client).await?;
    Ok(HttpResponse::Accepted().finish())
}

pub async fn check_existent_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("check_existent_connection: for tenant {:?}", ctx.tenant_id);

    let connection_id = id.into_inner();
    let tenant_id = ctx.tenant_id.clone();

    audit_data_connection(
        tenant_id.as_str(),
        connection_id.as_str(),
        service.meta_store.clone(),
        service.secret_store.clone(),
        &service.flight_client,
    )
    .await?;

    info!("Connection checked successfully");
    Ok(HttpResponse::NoContent().finish())
}

pub async fn test_credentials(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    body: web::Json<TestCredentials>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("test_credentials: for tenant {:?}", ctx.tenant_id);

    service
        .flight_client
        .test_credentials(&ctx.tenant_id, &body)
        .await
        .map_err(|e| ValidationError::ConnectionCheckFailed(e.message().to_string()))?;

    info!("Connection checked successfully");
    Ok(HttpResponse::NoContent().finish())
}

pub async fn export_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    parts: web::Path<(String, String)>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("export_connection: for tenant {:?}", ctx.tenant_id);
    let (id, secret_name) = parts.into_inner();

    let connection = service
        .meta_store
        .get_data_connection(ctx.tenant_id.as_str(), id.as_str())
        .await?;

    let mut props = HashMap::new();

    // Export the credentials into the new secret
    let existing_secret = service
        .secret_store
        .get_secret(&ctx.tenant_id, connection.resource.credentials_ref.secret.as_str())
        .await?;
    for (key, value) in existing_secret.properties.iter() {
        props.insert(key.to_string(), value.to_string());
    }

    props.insert("data_connection.id".to_string(), connection.metadata.id.to_string());
    props.insert(
        "data_connection_type.id".to_string(),
        connection.resource.data_connection_type_id.clone(),
    );
    props.insert("data_connection.name".to_string(), connection.resource.name.clone());
    props.insert(
        "data_connection.format".to_string(),
        connection.resource.format.to_string(),
    );
    for (key, value) in connection.resource.properties.iter() {
        props.insert(format!("data_connection.properties.{}", key), value.to_string());
    }

    let secret = Secret {
        name: secret_name,
        namespace: ctx.tenant_id.clone(),
        properties: props,
        labels: Some(default_secret_labels()),
        annotations: None,
    };

    service.secret_store.create_secret(&secret, true).await?;

    Ok(HttpResponse::NoContent().finish())
}

pub async fn not_found() -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::PathNotFound.into())
}

#[cfg(test)]
mod tests {
    use actix_web::{App, middleware, test, web};
    use commons::api::ResourceList;
    use commons::api::connection_types::DataConnectionTypeResource;
    use commons::api::connections::CredentialsRef;
    use commons::api::connections::DataConnectionResource;
    use commons::api::connections::DataConnectionStatus;
    use commons::api::errors::SecretStoreError;
    use commons::api::secret::Secret;
    use commons::api::storage::MetaStore;
    use commons::api::storage::SecretStore;
    use std::collections::HashMap;
    use std::sync::RwLock;

    use super::*;
    use crate::rest::API_VERSION;
    use crate::rest::errors::json_config;
    use crate::rest::middleware::validate_headers;

    fn api_path(path: &str) -> String {
        format!("/api/{API_VERSION}/data{path}")
    }

    struct StubMetaStore;

    #[async_trait::async_trait]
    impl MetaStore for StubMetaStore {
        async fn get_data_connections(
            &self,
            _t: &str,
        ) -> Result<ResourceList<DataConnectionResource>, commons::api::errors::MetaStoreError> {
            Ok(ResourceList {
                total_count: 0,
                items: vec![],
            })
        }

        async fn get_data_connection(
            &self,
            tenant_id: &str,
            uid: &str,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            if tenant_id == "test-tenant" && uid == "conn-1" {
                Ok(DataConnectionResource {
                    metadata: commons::api::ResourceMetadata {
                        id: "conn-1".to_string(),
                        tenant_id: Some("test-tenant".to_string()),
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        updated_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    resource: DataConnection {
                        name: "my-pg".to_string(),
                        data_connection_type_id: "ct-1".to_string(),
                        format: commons::api::connections::DataFormat::Tabular,
                        credentials_ref: CredentialsRef {
                            secret: "my-pg-creds".to_string(),
                        },
                        properties: HashMap::from([
                            ("host".to_string(), "localhost".to_string()),
                            ("port".to_string(), "5432".to_string()),
                        ]),
                    },
                    status: Default::default(),
                })
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "Data connection '{uid}' not found"
                )))
            }
        }

        async fn create_data_connection(
            &self,
            tenant_id: &str,
            data_connection: &DataConnection,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            if data_connection.data_connection_type_id != "ct-1" {
                return Err(commons::api::errors::MetaStoreError::UnprocessableEntity(format!(
                    "connection type '{}' not found",
                    data_connection.data_connection_type_id
                )));
            }
            Ok(DataConnectionResource {
                metadata: commons::api::ResourceMetadata {
                    id: "new-conn".to_string(),
                    tenant_id: Some(tenant_id.to_string()),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                },
                resource: data_connection.clone(),
                status: Default::default(),
            })
        }

        async fn update_data_connection(
            &self,
            tenant_id: &str,
            uid: &str,
            update_fn: Arc<
                dyn Fn(DataConnection) -> Result<DataConnection, commons::api::errors::MetaStoreError> + Send + Sync,
            >,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            if tenant_id == "test-tenant" && uid == "conn-1" {
                let existing = DataConnection {
                    name: "my-pg".to_string(),
                    data_connection_type_id: "ct-1".to_string(),
                    format: commons::api::connections::DataFormat::Tabular,
                    credentials_ref: CredentialsRef::default(),
                    properties: std::collections::HashMap::new(),
                };
                let updated = update_fn(existing)?;
                if updated.data_connection_type_id != "ct-1" {
                    return Err(commons::api::errors::MetaStoreError::UnprocessableEntity(format!(
                        "connection type '{}' not found",
                        updated.data_connection_type_id
                    )));
                }
                Ok(DataConnectionResource {
                    metadata: commons::api::ResourceMetadata {
                        id: "conn-1".to_string(),
                        tenant_id: Some("test-tenant".to_string()),
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        updated_at: "2026-01-02T00:00:00Z".to_string(),
                    },
                    resource: updated,
                    status: Default::default(),
                })
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "Data connection '{uid}' not found"
                )))
            }
        }

        async fn delete_data_connection(
            &self,
            tenant_id: &str,
            uid: &str,
        ) -> Result<(), commons::api::errors::MetaStoreError> {
            if tenant_id == "test-tenant" && uid == "conn-1" {
                Ok(())
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "Data connection '{uid}' not found"
                )))
            }
        }

        async fn update_data_connection_status(
            &self,
            _tenant_id: &str,
            _uid: &str,
            _update_fn: Arc<
                dyn Fn(DataConnectionStatus) -> Result<DataConnectionStatus, commons::api::errors::MetaStoreError>
                    + Send
                    + Sync,
            >,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }

        async fn get_data_connection_types(
            &self,
            _t: &str,
        ) -> Result<ResourceList<DataConnectionTypeResource>, commons::api::errors::MetaStoreError> {
            Ok(ResourceList {
                total_count: 0,
                items: vec![],
            })
        }

        async fn get_all_data_connection_types(
            &self,
        ) -> Result<ResourceList<DataConnectionTypeResource>, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }

        async fn get_data_connection_type(
            &self,
            tenant_id: &str,
            uid: &str,
        ) -> Result<DataConnectionTypeResource, commons::api::errors::MetaStoreError> {
            if tenant_id == "test-tenant" && uid == "ct-1" {
                Ok(DataConnectionTypeResource {
                    metadata: commons::api::ResourceMetadata {
                        id: "ct-1".to_string(),
                        tenant_id: Some("test-tenant".to_string()),
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        updated_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    resource: DataConnectionType {
                        name: "PostgreSQL".to_string(),
                        provider: "postgres".to_string(),
                        description: Some("PostgreSQL database connection".to_string()),
                        credentials_fields: vec![],
                    },
                    status: Default::default(),
                })
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "Data connection type '{uid}' not found"
                )))
            }
        }

        async fn create_data_connection_type(
            &self,
            tenant_id: &str,
            data_connection_type: &DataConnectionType,
        ) -> Result<DataConnectionTypeResource, commons::api::errors::MetaStoreError> {
            Ok(DataConnectionTypeResource {
                metadata: commons::api::ResourceMetadata {
                    id: "new-ct".to_string(),
                    tenant_id: Some(tenant_id.to_string()),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                },
                resource: data_connection_type.clone(),
                status: Default::default(),
            })
        }

        async fn update_data_connection_type(
            &self,
            tenant_id: &str,
            uid: &str,
            update_fn: Arc<
                dyn Fn(DataConnectionType) -> Result<DataConnectionType, commons::api::errors::MetaStoreError>
                    + Send
                    + Sync,
            >,
        ) -> Result<DataConnectionTypeResource, commons::api::errors::MetaStoreError> {
            if tenant_id == "test-tenant" && uid == "ct-1" {
                let existing = DataConnectionType {
                    name: "PostgreSQL".to_string(),
                    provider: "postgres".to_string(),
                    description: Some("PostgreSQL database connection".to_string()),
                    credentials_fields: vec![],
                };
                let updated = update_fn(existing)?;
                Ok(DataConnectionTypeResource {
                    metadata: commons::api::ResourceMetadata {
                        id: "ct-1".to_string(),
                        tenant_id: Some("test-tenant".to_string()),
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        updated_at: "2026-01-02T00:00:00Z".to_string(),
                    },
                    resource: updated,
                    status: Default::default(),
                })
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "Data connection type '{uid}' not found"
                )))
            }
        }

        async fn update_data_connection_type_status(
            &self,
            uid: &str,
            update_fn: Arc<
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
            let (metadata, resource, current_status) =
                if let Ok(dct) = self.get_data_connection_type("test-tenant", uid).await {
                    (dct.metadata, dct.resource, dct.status)
                } else {
                    (
                        commons::api::ResourceMetadata {
                            id: uid.to_string(),
                            tenant_id: Some("test-tenant".to_string()),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            updated_at: "2026-01-01T00:00:00Z".to_string(),
                        },
                        DataConnectionType {
                            name: String::new(),
                            provider: String::new(),
                            description: None,
                            credentials_fields: vec![],
                        },
                        Default::default(),
                    )
                };
            let status = update_fn(current_status)?;
            Ok(DataConnectionTypeResource {
                metadata,
                resource,
                status,
            })
        }

        async fn delete_data_connection_type(
            &self,
            tenant_id: &str,
            uid: &str,
        ) -> Result<(), commons::api::errors::MetaStoreError> {
            if tenant_id == "test-tenant" && uid == "ct-1" {
                Ok(())
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "Data connection type '{uid}' not found"
                )))
            }
        }
    }

    struct StubSecretStore {
        secrets: RwLock<HashMap<String, Secret>>,
    }

    impl StubSecretStore {
        fn new() -> Self {
            let mut secrets = HashMap::new();
            secrets.insert(
                "test-tenant/my-pg-creds".to_string(),
                Secret {
                    name: "my-pg-creds".to_string(),
                    namespace: "test-tenant".to_string(),
                    properties: HashMap::from([
                        ("username".to_string(), "pg_user".to_string()),
                        ("password".to_string(), "pg_pass".to_string()),
                    ]),
                    labels: None,
                    annotations: None,
                },
            );
            Self {
                secrets: RwLock::new(secrets),
            }
        }
    }

    #[async_trait::async_trait]
    impl SecretStore for StubSecretStore {
        async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError> {
            let secrets = self.secrets.read().unwrap();
            secrets
                .get(&format!("{namespace}/{name}"))
                .cloned()
                .ok_or(SecretStoreError::SecretNotFound(format!("{namespace}/{name}")))
        }
        async fn create_secret(&self, secret: &Secret, _overwrite: bool) -> Result<(), SecretStoreError> {
            let mut secrets = self.secrets.write().unwrap();
            secrets.insert(format!("{}/{}", secret.namespace, secret.name), secret.clone());
            Ok(())
        }
        async fn delete_secret(&self, _n: &str, _k: &str) -> Result<(), SecretStoreError> {
            unimplemented!()
        }
        async fn set_secret_labels(
            &self,
            _n: &str,
            _k: &str,
            _l: HashMap<String, String>,
        ) -> Result<(), SecretStoreError> {
            unimplemented!()
        }
    }

    fn test_service() -> web::Data<ApiService> {
        web::Data::new(ApiService::new(
            Arc::new(StubMetaStore),
            Arc::new(StubSecretStore::new()),
            FlightClient::new("http://localhost:50051".to_string()),
        ))
    }

    fn test_app_config(cfg: &mut web::ServiceConfig) {
        cfg.service(
            web::scope(&format!("/api/{API_VERSION}/data"))
                .wrap(middleware::from_fn(validate_headers))
                .route("/connections", web::get().to(list_connections))
                .route("/connections", web::post().to(create_connection))
                .route("/connections/{id}", web::get().to(get_connection))
                .route("/connections/{id}", web::patch().to(patch_connection))
                .route("/connections/{id}", web::delete().to(delete_connection))
                .route("/connection-types", web::get().to(list_connection_types))
                .route("/connection-types", web::post().to(create_connection_type))
                .route("/connection-types/{id}", web::get().to(get_connection_type))
                .route("/connection-types/{id}", web::patch().to(patch_connection_type))
                .route("/connection-types/{id}", web::delete().to(delete_connection_type))
                .route("/ingestion/{id}", web::get().to(get_ingestion_data))
                .route(
                    "/connections/{id}/exports/secrets/{secret_name}",
                    web::put().to(export_connection),
                )
                .default_service(web::route().to(not_found)),
        );
    }

    #[actix_web::test]
    async fn test_health() {
        let app = test::init_service(App::new().route("/health", web::get().to(health))).await;
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn test_not_found() {
        let app = test::init_service(
            App::new()
                .configure(test_app_config)
                .default_service(web::route().to(not_found)),
        )
        .await;
        let req = test::TestRequest::get().uri("/anything").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "path_not_found");
        assert_eq!(body["message"], "Path not found");
    }

    #[actix_web::test]
    async fn test_list_connections() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connections"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["total_count"], 0);
        assert_eq!(body["items"], serde_json::json!([]));
    }

    #[actix_web::test]
    async fn test_get_connection() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connections/conn-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["metadata"]["id"], "conn-1");
        assert_eq!(body["resource"]["name"], "my-pg");
    }

    #[actix_web::test]
    async fn test_get_connection_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connections/nonexistent"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "not_found");
    }

    #[actix_web::test]
    async fn test_create_connection() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::post()
            .uri(&api_path("/connections"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .insert_header(("content-type", "application/json"))
            .set_json(serde_json::json!({
                "name": "my-pg",
                "data_connection_type_id": "ct-1",
                "format": "tabular",
                "credentials_ref": {
                    "secret": "my-pg-creds"
                },
                "properties": {}
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["metadata"]["id"], "new-conn");
        assert_eq!(body["metadata"]["tenant_id"], "test-tenant");
        assert_eq!(body["resource"]["name"], "my-pg");
    }

    #[actix_web::test]
    async fn test_create_connection_nonexistent_type() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::post()
            .uri(&api_path("/connections"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .insert_header(("content-type", "application/json"))
            .set_json(serde_json::json!({
                "name": "my-pg",
                "data_connection_type_id": "nonexistent-type-id",
                "format": "tabular",
                "credentials_ref": {
                    "secret": "my-pg-creds"
                },
                "properties": {}
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 422);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "unprocessable_entity");
    }

    #[actix_web::test]
    async fn test_patch_connection_replace_name() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::patch()
            .uri(&api_path("/connections/conn-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .set_json(serde_json::json!({"name": "renamed-pg"}))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["metadata"]["id"], "conn-1");
        assert_eq!(body["resource"]["name"], "renamed-pg");
        assert_eq!(body["resource"]["data_connection_type_id"], "ct-1");
    }

    #[actix_web::test]
    async fn test_patch_connection_add_property() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::patch()
            .uri(&api_path("/connections/conn-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .set_json(serde_json::json!({"properties": {"host": "localhost"}}))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["resource"]["properties"]["host"], "localhost");
    }

    #[actix_web::test]
    async fn test_patch_connection_not_found() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::patch()
            .uri(&api_path("/connections/nonexistent"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .set_json(serde_json::json!({"name": "x"}))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "not_found");
    }

    #[actix_web::test]
    async fn test_patch_connection_nonexistent_type() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::patch()
            .uri(&api_path("/connections/conn-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .set_json(serde_json::json!({"data_connection_type_id": "nonexistent-type-id"}))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 422);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "unprocessable_entity");
    }

    #[actix_web::test]
    async fn test_delete_connection() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&api_path("/connections/conn-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 204);
    }

    #[actix_web::test]
    async fn test_delete_connection_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&api_path("/connections/nonexistent"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "not_found");
    }

    #[actix_web::test]
    async fn test_get_connection_cross_tenant() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connections/conn-1"))
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_delete_connection_cross_tenant() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&api_path("/connections/conn-1"))
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_delete_connection_type_cross_tenant() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&api_path("/connection-types/ct-1"))
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_missing_tenant_header() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get().uri(&api_path("/connections")).to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "header_not_found");
    }

    #[actix_web::test]
    async fn test_list_connection_types() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connection-types"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["total_count"], 0);
        assert_eq!(body["items"], serde_json::json!([]));
    }

    #[actix_web::test]
    async fn test_create_connection_type() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::post()
            .uri(&api_path("/connection-types"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .insert_header(("content-type", "application/json"))
            .set_json(serde_json::json!({
                "name": "PostgreSQL",
                "provider": "postgres",
                "description": "PostgreSQL database connection",
                "credentials_fields": []
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["metadata"]["id"], "new-ct");
        assert_eq!(body["metadata"]["tenant_id"], "test-tenant");
        assert_eq!(body["resource"]["name"], "PostgreSQL");
        assert_eq!(body["resource"]["provider"], "postgres");
    }

    #[actix_web::test]
    async fn test_get_connection_type() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connection-types/ct-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["metadata"]["id"], "ct-1");
        assert_eq!(body["resource"]["name"], "PostgreSQL");
        assert_eq!(body["resource"]["provider"], "postgres");
    }

    #[actix_web::test]
    async fn test_get_connection_type_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connection-types/nonexistent"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "not_found");
    }

    #[actix_web::test]
    async fn test_get_connection_type_cross_tenant() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connection-types/ct-1"))
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_get_ingestion_data_unimplemented() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri(&api_path("/ingestion/some-id"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 501);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "unimplemented");
        assert_eq!(body["message"], "Unimplemented");
    }

    #[actix_web::test]
    async fn test_patch_connection_type_replace_name() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::patch()
            .uri(&api_path("/connection-types/ct-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .set_json(serde_json::json!({"name": "MySQL"}))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["metadata"]["id"], "ct-1");
        assert_eq!(body["resource"]["name"], "MySQL");
        assert_eq!(body["resource"]["provider"], "postgres");
    }

    #[actix_web::test]
    async fn test_patch_connection_type_not_found() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::patch()
            .uri(&api_path("/connection-types/nonexistent"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .set_json(serde_json::json!({"name": "x"}))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "not_found");
    }

    #[actix_web::test]
    async fn test_delete_connection_type() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&api_path("/connection-types/ct-1"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 204);
    }

    #[actix_web::test]
    async fn test_delete_connection_type_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&api_path("/connection-types/nonexistent"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "not_found");
    }

    #[actix_web::test]
    async fn test_invalid_json_body() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::post()
            .uri(&api_path("/connections"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .insert_header(("content-type", "application/json"))
            .set_payload("not json")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "invalid_json");
    }

    #[actix_web::test]
    async fn test_export_connection() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::put()
            .uri(&api_path("/connections/conn-1/exports/secrets/exported-secret"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 204);
    }

    #[actix_web::test]
    async fn test_export_connection_includes_connection_fields() {
        let svc = test_service();
        let app = test::init_service(App::new().app_data(svc.clone()).configure(test_app_config)).await;
        let req = test::TestRequest::put()
            .uri(&api_path("/connections/conn-1/exports/secrets/exported-secret"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        test::call_service(&app, req).await;

        let secret_store = svc.secret_store.clone();
        let secret = secret_store
            .get_secret("test-tenant", "exported-secret")
            .await
            .expect("exported secret should exist");

        assert_eq!(secret.properties["data_connection.id"], "conn-1");
        assert_eq!(secret.properties["data_connection.name"], "my-pg");
        assert_eq!(secret.properties["data_connection_type.id"], "ct-1");
        assert_eq!(secret.properties["data_connection.format"], "tabular");
        assert_eq!(secret.properties["data_connection.properties.host"], "localhost");
        assert_eq!(secret.properties["data_connection.properties.port"], "5432");
    }

    #[actix_web::test]
    async fn test_export_connection_includes_credentials() {
        let svc = test_service();
        let app = test::init_service(App::new().app_data(svc.clone()).configure(test_app_config)).await;
        let req = test::TestRequest::put()
            .uri(&api_path("/connections/conn-1/exports/secrets/exported-secret"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        test::call_service(&app, req).await;

        let secret = svc
            .secret_store
            .get_secret("test-tenant", "exported-secret")
            .await
            .expect("exported secret should exist");

        assert_eq!(secret.properties["username"], "pg_user");
        assert_eq!(secret.properties["password"], "pg_pass");
    }

    #[actix_web::test]
    async fn test_export_connection_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::put()
            .uri(&api_path("/connections/nonexistent/exports/secrets/exported-secret"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_export_connection_cross_tenant() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::put()
            .uri(&api_path("/connections/conn-1/exports/secrets/exported-secret"))
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }
}
