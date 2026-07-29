use actix_web::{HttpResponse, Responder, web};

pub async fn list_connections(path: Option<web::Path<String>>) -> impl Responder {
    let namespace = path.map(|p| p.into_inner());
    HttpResponse::Ok().body(format!("Listing connections for namespace: {:?}", namespace))
}

pub async fn get_connection(path: web::Path<(String, String)>) -> impl Responder {
    let (namespace, name) = path.into_inner();
    HttpResponse::Ok().body(format!("{}:{}", namespace, name))
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
                .service(web::resource("/connections").to(list_connections))
                .service(web::resource("/connections/{namespace}").to(list_connections))
                .service(web::resource("/connections/{namespace}/{name}").to(get_connection)),
        );
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

    #[actix_web::test]
    async fn test_list_connections_with_namespace() {
        let app = test::init_service(App::new().configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri("/v1/data/connections/my-namespace")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body = test::read_body(resp).await;
        assert_eq!(body, "Listing connections for namespace: Some(\"my-namespace\")");
    }

    #[actix_web::test]
    async fn test_get_connection() {
        let app = test::init_service(App::new().configure(test_app_config)).await;
        let req = test::TestRequest::get()
            .uri("/v1/data/connections/my-namespace/my-connection")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 200);
        let body = test::read_body(resp).await;
        assert_eq!(body, "my-namespace:my-connection");
    }
}
