use super::JsonPatch;
use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use actix_web::web::Bytes;
use actix_web::{HttpResponse, web};
use commons::api::connections::DataConnection;
use commons::api::connections::MetaStore;
use commons::api::connections::SecretStore;
use serde::Serialize;
use std::sync::Arc;
use tracing::info;

use commons::api::connections::DataConnectionType;

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
    _secret_store: Arc<dyn SecretStore + Send + Sync>,
}

impl ApiService {
    pub fn new(meta_store: Arc<dyn MetaStore + Send + Sync>, secret_store: Arc<dyn SecretStore + Send + Sync>) -> Self {
        Self {
            meta_store,
            _secret_store: secret_store,
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
    _service: web::Data<ApiService>,
    _ctx: web::ReqData<ApiContext>,
    _id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn create_connection(
    _service: web::Data<ApiService>,
    _ctx: web::ReqData<ApiContext>,
    connection: web::Json<DataConnection>,
) -> Result<HttpResponse, RestErrorResponse> {
    Ok(HttpResponse::Created().json(connection))
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
    let id = id.clone();
    let connection_type = service
        .meta_store
        .get_data_connection_type(ctx.tenant_id.as_str(), id.as_str())
        .await?;
    Ok(HttpResponse::Ok().json(connection_type))
}

pub async fn patch_connection(
    _service: web::Data<ApiService>,
    _ctx: web::ReqData<ApiContext>,
    _id: web::Path<String>,
    _body: web::Json<Vec<JsonPatch>>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
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
    _service: web::Data<ApiService>,
    _ctx: web::ReqData<ApiContext>,
    _path: web::Path<String>,
    _body: Bytes,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn delete_connection(
    _service: web::Data<ApiService>,
    _ctx: web::ReqData<ApiContext>,
    _path: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn delete_connection_type(
    _service: web::Data<ApiService>,
    _ctx: web::ReqData<ApiContext>,
    _path: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn not_found() -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::PathNotFound.into())
}

#[cfg(test)]
mod tests {
    use actix_web::{App, middleware, test, web};
    use commons::api::ResourceList;
    use commons::api::connections::{DataConnectionResource, DataConnectionTypeResource, Secret};
    use commons::api::errors::SecretStoreError;

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
            _t: &str,
            _u: &str,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
        async fn create_data_connection(
            &self,
            _t: &str,
            _d: &DataConnection,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
        async fn update_data_connection(
            &self,
            _t: &str,
            _u: &str,
            _f: Arc<
                dyn Fn(DataConnection) -> Result<DataConnection, commons::api::errors::MetaStoreError> + Send + Sync,
            >,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
        async fn delete_data_connection(&self, _t: &str, _u: &str) -> Result<(), commons::api::errors::MetaStoreError> {
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
        async fn get_data_connection_type(
            &self,
            _t: &str,
            _i: &str,
        ) -> Result<DataConnectionTypeResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
        async fn create_data_connection_type(
            &self,
            _t: &str,
            _d: &DataConnectionType,
        ) -> Result<DataConnectionTypeResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
        async fn update_data_connection_type(
            &self,
            _t: &str,
            _u: &str,
            _f: Arc<
                dyn Fn(DataConnectionType) -> Result<DataConnectionType, commons::api::errors::MetaStoreError>
                    + Send
                    + Sync,
            >,
        ) -> Result<DataConnectionTypeResource, commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
        async fn delete_data_connection_type(
            &self,
            _t: &str,
            _u: &str,
        ) -> Result<(), commons::api::errors::MetaStoreError> {
            unimplemented!()
        }
    }

    struct StubSecretStore;

    #[async_trait::async_trait]
    impl SecretStore for StubSecretStore {
        async fn get_secret(&self, _n: &str, _k: &str) -> Result<Secret, SecretStoreError> {
            unimplemented!()
        }
    }

    fn test_service() -> web::Data<ApiService> {
        web::Data::new(ApiService::new(Arc::new(StubMetaStore), Arc::new(StubSecretStore)))
    }

    fn test_app_config(cfg: &mut web::ServiceConfig) {
        cfg.service(
            web::scope("/api/v1/data")
                .wrap(middleware::from_fn(validate_headers))
                .route("/connections", web::get().to(list_connections))
                .route("/connections", web::post().to(create_connection))
                .route("/connections/{id}", web::get().to(get_connection))
                .route("/connection-types", web::get().to(list_connection_types))
                .route("/connection-types/{id}", web::get().to(get_connection_type))
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
