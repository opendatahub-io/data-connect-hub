use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use super::errors::ValidationError;
use crate::clients::flight::FlightClient;
use crate::state::audit::audit_data_connection_types;
use crate::utils::transform_data_connection;
use actix_web::{HttpResponse, web};
use chrono::Utc;
use commons::api::connection_types::DataConnectionType;
use commons::api::connections::{DataConnection, DataConnectionState, DataConnectionStatus};
use commons::api::creds::TestCredentials;
use commons::api::storage::MetaStore;
use commons::api::storage::SecretStore;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

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

pub async fn create_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    connection: web::Json<DataConnection>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("create_connection: for tenant {:?}", ctx.tenant_id);
    let tenant_id = ctx.tenant_id.clone();

    let connection = transform_data_connection(&tenant_id, &connection).await;

    let connection_res = service
        .meta_store
        .create_data_connection(ctx.tenant_id.as_str(), &connection.0)
        .await?;

    if let Some(secret) = connection.1 {
        let secret = &mut secret.clone();
        secret.labels = Arc::new(HashMap::from([(
            "dataconnecthub.opendatahub.io/attached".to_string(),
            "true".to_string(),
        )]));

        service.secret_store.create_secret(secret).await?;
    }

    Ok(HttpResponse::Created().json(connection_res))
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

    let result = service
        .flight_client
        .check_connection(&ctx.tenant_id, &connection_id)
        .await;

    match result {
        Ok(_) => {
            let update_fn = Arc::new(|_: DataConnectionStatus| {
                let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                Ok(DataConnectionStatus {
                    state: DataConnectionState::Ready,
                    message: Some("Connection check successful".to_string()),
                    updated_at: Some(now),
                    phases: vec![],
                })
            });
            service
                .meta_store
                .update_data_connection_status(&connection_id, update_fn)
                .await?;
        },
        Err(_) => {
            let update_fn = Arc::new(|_: DataConnectionStatus| {
                let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                Ok(DataConnectionStatus {
                    state: DataConnectionState::NotReady,
                    message: Some("Connection check failed".to_string()),
                    updated_at: Some(now),
                    phases: vec![],
                })
            });
            service
                .meta_store
                .update_data_connection_status(&connection_id, update_fn)
                .await?;

            return Err(ValidationError::ConnectionCheckFailed(connection_id).into());
        },
    };

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

pub async fn not_found() -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::PathNotFound.into())
}

#[cfg(test)]
mod tests {
    use actix_web::{App, middleware, test, web};
    use commons::api::ResourceList;
    use commons::api::connection_types::DataConnectionTypeResource;
    use commons::api::connection_types::Secret;
    use commons::api::connections::DataConnectionResource;
    use commons::api::errors::SecretStoreError;
    use commons::api::storage::MetaStore;
    use commons::api::storage::SecretStore;
    use std::collections::HashMap;

    use super::*;
    use crate::rest::errors::json_config;
    use crate::rest::middleware::validate_headers;

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
                        admin: None,
                        properties: std::collections::HashMap::new(),
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
                    admin: None,
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
            _uid: &str,
            _update_fn: Arc<
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

    struct StubSecretStore;

    #[async_trait::async_trait]
    impl SecretStore for StubSecretStore {
        async fn get_secret(&self, _n: &str, _k: &str) -> Result<Secret, SecretStoreError> {
            unimplemented!()
        }
        async fn create_secret(&self, _s: &Secret) -> Result<(), SecretStoreError> {
            unimplemented!()
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
            Arc::new(StubSecretStore),
            FlightClient::new("http://localhost:50051".to_string()),
        ))
    }

    fn test_app_config(cfg: &mut web::ServiceConfig) {
        cfg.service(
            web::scope("/api/v1/data")
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
            .uri("/api/v1/data/connections")
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
            .uri("/api/v1/data/connections/conn-1")
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
            .uri("/api/v1/data/connections/nonexistent")
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
            .uri("/api/v1/data/connections")
            .insert_header(("x-tenant-id", "test-tenant"))
            .insert_header(("content-type", "application/json"))
            .set_json(serde_json::json!({
                "name": "my-pg",
                "data_connection_type_id": "ct-1",
                "format": "tabular",
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
            .uri("/api/v1/data/connections")
            .insert_header(("x-tenant-id", "test-tenant"))
            .insert_header(("content-type", "application/json"))
            .set_json(serde_json::json!({
                "name": "my-pg",
                "data_connection_type_id": "nonexistent-type-id",
                "format": "tabular",
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
            .uri("/api/v1/data/connections/conn-1")
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
            .uri("/api/v1/data/connections/conn-1")
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
            .uri("/api/v1/data/connections/nonexistent")
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
            .uri("/api/v1/data/connections/conn-1")
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
            .uri("/api/v1/data/connections/conn-1")
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 204);
    }

    #[actix_web::test]
    async fn test_delete_connection_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri("/api/v1/data/connections/nonexistent")
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
            .uri("/api/v1/data/connections/conn-1")
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_delete_connection_cross_tenant() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri("/api/v1/data/connections/conn-1")
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_delete_connection_type_cross_tenant() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri("/api/v1/data/connection-types/ct-1")
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_missing_tenant_header() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get().uri("/api/v1/data/connections").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "header_not_found");
    }

    #[actix_web::test]
    async fn test_list_connection_types() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri("/api/v1/data/connection-types")
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
            .uri("/api/v1/data/connection-types")
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
            .uri("/api/v1/data/connection-types/ct-1")
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
            .uri("/api/v1/data/connection-types/nonexistent")
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
            .uri("/api/v1/data/connection-types/ct-1")
            .insert_header(("x-tenant-id", "other-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_get_ingestion_data_unimplemented() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri("/api/v1/data/ingestion/some-id")
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
            .uri("/api/v1/data/connection-types/ct-1")
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
            .uri("/api/v1/data/connection-types/nonexistent")
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
            .uri("/api/v1/data/connection-types/ct-1")
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 204);
    }

    #[actix_web::test]
    async fn test_delete_connection_type_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_app_config)).await;
        let req = test::TestRequest::delete()
            .uri("/api/v1/data/connection-types/nonexistent")
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
            .uri("/api/v1/data/connections")
            .insert_header(("x-tenant-id", "test-tenant"))
            .insert_header(("content-type", "application/json"))
            .set_payload("not json")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "invalid_json");
    }
}
