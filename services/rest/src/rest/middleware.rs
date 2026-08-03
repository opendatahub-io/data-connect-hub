use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{HttpMessage, HttpResponse};

use super::endpoints::AppData;
use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use commons::api::X_TENANT_ID;

fn error_response(err: EndpointError) -> HttpResponse {
    let error: RestErrorResponse = err.into();
    HttpResponse::BadRequest().json(&error)
}

pub async fn validate_headers(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    let tenant_id = match req.headers().get(X_TENANT_ID) {
        Some(value) => match value.to_str() {
            Ok(v) if !v.is_empty() => v.to_string(),
            Ok(_) => {
                return Ok(req
                    .into_response(error_response(EndpointError::InvalidHeaderValue(
                        X_TENANT_ID.to_string(),
                    )))
                    .map_into_right_body());
            },
            Err(_) => {
                return Ok(req
                    .into_response(error_response(EndpointError::InvalidHeaderValue(
                        X_TENANT_ID.to_string(),
                    )))
                    .map_into_right_body());
            },
        },
        None => {
            return Ok(req
                .into_response(error_response(EndpointError::HeaderNotFound(X_TENANT_ID.to_string())))
                .map_into_right_body());
        },
    };

    req.extensions_mut().insert(AppData { tenant_id });

    next.call(req).await.map(ServiceResponse::map_into_left_body)
}
