use actix_web::{HttpResponse, Responder, web};
use commons::api::connections::{DataConnection, DataConnectionType};

pub async fn health() -> impl Responder {
    HttpResponse::Ok().finish()
}

pub async fn list_connections(path: Option<web::Path<String>>) -> impl Responder {
    let namespace = path.map(|p| p.into_inner());
    HttpResponse::Ok().body(format!("Listing connections for namespace: {:?}", namespace))
}

pub async fn get_connection(path: web::Path<(String, String)>) -> impl Responder {
    let (namespace, name) = path.into_inner();
    HttpResponse::Ok().body(format!("{}:{}", namespace, name))
}

pub async fn list_connection_types() -> impl Responder {
    HttpResponse::Ok().body("Listing connection types")
}

pub async fn get_connection_type(path: web::Path<String>) -> impl Responder {
    let id = path.into_inner();
    HttpResponse::Ok().body(id)
}

pub async fn create_connection(_body: web::Json<DataConnection>) -> impl Responder {
    HttpResponse::Ok().body("Creating connection")
}

pub async fn patch_connection(path: web::Path<(String, String)>, _body: web::Json<DataConnection>) -> impl Responder {
    let (namespace, name) = path.into_inner();
    HttpResponse::Ok().body(format!("{}:{}", namespace, name))
}

pub async fn create_connection_type(_body: web::Json<DataConnectionType>) -> impl Responder {
    HttpResponse::Ok().body("Creating connection type")
}

pub async fn patch_connection_type(path: web::Path<String>, _body: web::Json<DataConnectionType>) -> impl Responder {
    let id = path.into_inner();
    HttpResponse::Ok().body(id)
}

pub async fn delete_connection(path: web::Path<String>) -> impl Responder {
    let id = path.into_inner();
    HttpResponse::Ok().body(id)
}

pub async fn delete_connection_type(path: web::Path<String>) -> impl Responder {
    let id = path.into_inner();
    HttpResponse::Ok().body(id)
}

pub async fn not_found() -> impl Responder {
    HttpResponse::NotFound().body("Not Found")
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
        let body = test::read_body(resp).await;
        assert_eq!(body, "Not Found");
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
