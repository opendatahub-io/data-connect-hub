use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{HttpMessage, HttpResponse};

use super::endpoints::AppData;
use super::errors::EndpointError;
use super::errors::RestError;
use commons::api::X_TENANT_ID;

pub async fn validate_headers(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    let tenant_id = match req.headers().get(X_TENANT_ID) {
        Some(value) => match value.to_str() {
            Ok(v) => v.to_string(),
            Err(_) => {
                let error: RestError = EndpointError::InvalidHeaderValue(X_TENANT_ID.to_string()).into();
                let response = HttpResponse::BadRequest().json(&error);
                return Ok(req.into_response(response).map_into_right_body());
            },
        },
        None => {
            let error: RestError = EndpointError::HeaderNotFound(X_TENANT_ID.to_string()).into();
            let response = HttpResponse::BadRequest().json(&error);
            return Ok(req.into_response(response).map_into_right_body());
        },
    };

    req.extensions_mut().insert(AppData { tenant_id });

    next.call(req).await.map(ServiceResponse::map_into_left_body)
}
