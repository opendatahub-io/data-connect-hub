use super::errors::EndpointError;
use super::errors::RestError;
use actix_web::web::Bytes;
use actix_web::{HttpResponse, web};
use commons::api::connections::{DataConnection, DataConnectionType};

#[derive(Clone)]
pub struct AppData {
    pub tenant_id: String,
}

pub async fn health() -> Result<HttpResponse, RestError> {
    Ok(HttpResponse::Ok().finish())
}

pub async fn list_connections(
    _app_data: web::ReqData<AppData>,
    _path: Option<web::Path<String>>,
) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn get_connection(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<(String, String)>,
) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn list_connection_types(_app_data: web::ReqData<AppData>) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn get_connection_type(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<String>,
) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn create_connection(app_data: web::ReqData<AppData>, body: Bytes) -> Result<HttpResponse, RestError> {
    let _tenant_id = app_data.tenant_id.clone();
    let connection: DataConnection = serde_json::from_slice(&body).map_err(|_| EndpointError::InvalidPayload)?;

    Ok(HttpResponse::Ok().json(connection))
}

pub async fn patch_connection(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<String>,
    _body: Bytes,
) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn create_connection_type(_body: web::Json<DataConnectionType>) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn patch_connection_type(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<String>,
    _body: Bytes,
) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn delete_connection(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<String>,
) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn delete_connection_type(
    _app_data: web::ReqData<AppData>,
    _path: web::Path<String>,
) -> Result<HttpResponse, RestError> {
    Err(EndpointError::Unimplemented.into())
}

pub async fn not_found() -> Result<HttpResponse, RestError> {
    Err(EndpointError::PathNotFound.into())
}

#[cfg(test)]
mod tests {
    use actix_web::{App, test, web};

    use super::*;

    fn test_app_config(cfg: &mut web::ServiceConfig) {
        cfg.service(
            web::scope("/v1/data")
                .route("/connections", web::get().to(list_connections))
                .route("/connections/{id}", web::get().to(get_connection))
                .route("/connection_types", web::get().to(list_connection_types))
                .route("/connection_types/{id}", web::get().to(get_connection_type))
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
    async fn test_list_connections_no_namespace() {
        let app = test::init_service(App::new().configure(test_app_config)).await;
        let req = test::TestRequest::get().uri("/v1/data/connections").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body = test::read_body(resp).await;
        assert_eq!(body, "Listing connections for namespace: None");
    }
}
