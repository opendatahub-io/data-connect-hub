use super::JsonPatch;
use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use actix_web::web::Bytes;
use actix_web::{HttpResponse, web};
use commons::api::connections::DataConnection;
use serde::Serialize;

#[derive(Clone)]
pub struct AppData {
    pub tenant_id: String,
}

#[derive(Serialize)]
struct HealthResponse {
    service: String,
}

pub async fn health() -> Result<HttpResponse, RestErrorResponse> {
    Ok(HttpResponse::Ok().json(HealthResponse {
        service: "Data Connect Hub".to_string(),
    }))
}

pub async fn list_connections(_app_data: web::ReqData<AppData>) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn get_connection(
    _app_data: web::ReqData<AppData>,
    _id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn list_connection_types(_app_data: web::ReqData<AppData>) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn get_connection_type(
    _app_data: web::ReqData<AppData>,
    _id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn create_connection(
    app_data: web::ReqData<AppData>,
    connection: web::Json<DataConnection>,
) -> Result<HttpResponse, RestErrorResponse> {
    let _tenant_id = app_data.tenant_id.clone();

    Ok(HttpResponse::Ok().json(connection))
}

pub async fn patch_connection(
    _app_data: web::ReqData<AppData>,
    _id: web::Path<String>,
    _body: web::Json<Vec<JsonPatch>>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn create_connection_type(
    _app_data: web::ReqData<AppData>,
    _body: Bytes,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn patch_connection_type(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<String>,
    _body: Bytes,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn delete_connection(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn delete_connection_type(
    _app_data: web::ReqData<AppData>,
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

    use super::*;
    use crate::rest::errors::json_config;
    use crate::rest::middleware::validate_headers;

    fn test_app_config(cfg: &mut web::ServiceConfig) {
        cfg.service(
            web::scope("/v1/data")
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
    async fn test_list_connections_unimplemented() {
        let app = test::init_service(App::new().configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri("/v1/data/connections")
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 501);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "unimplemented");
    }

    #[actix_web::test]
    async fn test_missing_tenant_header() {
        let app = test::init_service(App::new().configure(test_app_config)).await;
        let req = test::TestRequest::get().uri("/v1/data/connections").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "header_not_found");
    }

    #[actix_web::test]
    async fn test_invalid_json_body() {
        let app = test::init_service(App::new().app_data(json_config()).configure(test_app_config)).await;
        let req = test::TestRequest::post()
            .uri("/v1/data/connections")
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
