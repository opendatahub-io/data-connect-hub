use super::errors::EndpointError;
use super::errors::RestErrorResponse;
use super::errors::ValidationError;

use crate::state::audit::audit_data_connection;
use crate::state::audit::audit_data_connection_types;
use crate::utils::default_secret_labels;
use actix_web::{HttpResponse, web};
use arrow::array::{Array, AsArray};
use commons::api::connection_types::DataConnectionType;
use commons::api::connections::DataConnection;
use commons::api::creds::TestCredentials;
use commons::api::errors::MetaStoreError;
use commons::api::flight_discovery::FlightService;
use commons::api::flight_discovery::FlightServiceResource;
use commons::api::secret::Secret;
use commons::api::storage::MetaStore;
use commons::api::storage::SecretStore;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::error;
use tracing::info;

use crate::rest::CreateConnectionRequest;
use crate::rest::DataConnectionWithCreds;
use crate::state::ApiService;

#[derive(Clone)]
pub struct ApiContext {
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
    info!(
        "create_connection_type: {} from provider {} (tenant {})",
        connection_type.name, connection_type.provider, ctx.tenant_id,
    );

    let connection_type = service
        .meta_store
        .create_data_connection_type(ctx.tenant_id.as_str(), &connection_type)
        .await?;

    audit_data_connection_types(service.as_ref()).await?;

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

    audit_data_connection_types(service.as_ref()).await?;

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

#[derive(Deserialize)]
pub struct BinaryDownloadQuery {
    pub path: String,
}

pub async fn get_binary_data(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
    query: web::Query<BinaryDownloadQuery>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("get_binary_data: binary download for tenant {:?}", ctx.tenant_id);

    let flight = flight_service_by_connection(service.as_ref(), ctx.tenant_id.as_str(), id.as_str()).await?;

    let batch_stream = service
        .flight_client(flight.resource.internal_url.as_str())
        .download_binary(&ctx.tenant_id, &id, &query.path)
        .await?;

    let body_stream = batch_stream.map(|result| match result {
        Ok(batch) => {
            let array = batch
                .column_by_name("data")
                .ok_or_else(|| actix_web::error::ErrorInternalServerError("missing 'data' column in response"))?;
            let binary_array = array.as_binary::<i32>();
            let mut data = Vec::new();
            for i in 0..binary_array.len() {
                data.extend_from_slice(binary_array.value(i));
            }
            Ok(web::Bytes::from(data))
        },
        Err(e) => {
            error!(error = %e, "error reading binary stream from flight service");
            Err(actix_web::error::ErrorInternalServerError("binary stream read failed"))
        },
    });

    let filename = query.path.rsplit('/').next().unwrap_or("download");

    Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .insert_header(("Content-Disposition", format!("attachment; filename=\"{filename}\"")))
        .streaming(body_stream))
}

pub async fn create_flight_service(
    service: web::Data<ApiService>,
    body: web::Json<FlightService>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("audit_connection_types");

    let api_service = service.as_ref();
    let mut flight = body.into_inner();

    let connectors = service
        .flight_client(&flight.internal_url)
        .get_supported_connectors(&service.global_tenant_id)
        .await?;

    let connectors_names = connectors.iter().map(|c| c.name.clone()).collect::<Vec<_>>();

    let flight_services = service.meta_store.get_all_flight_services().await?;

    let existent_connectors = flight_services
        .items
        .iter()
        .flat_map(|fs| fs.resource.supported_connectors.clone())
        .collect::<HashSet<_>>();

    let intersection = connectors_names
        .iter()
        .filter(|fs| existent_connectors.contains(&fs.to_string()))
        .collect::<Vec<_>>();

    if !intersection.is_empty() {
        return Err(
            ValidationError::ConnectorsAlreadyExists(format!("connectors already exists: {:?}", intersection)).into(),
        );
    }

    flight.supported_connectors = connectors.into_iter().map(|c| c.name).collect();

    let res = api_service.meta_store.create_flight_service(&flight).await?;

    audit_data_connection_types(api_service).await?;
    Ok(HttpResponse::Created().json(res))
}

pub async fn delete_flight_service(
    service: web::Data<ApiService>,
    parts: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    let id = parts.into_inner();
    info!("delete_flight_service: id={id}");

    service.meta_store.delete_flight_service(&id).await?;
    audit_data_connection_types(service.as_ref()).await?;

    Ok(HttpResponse::NoContent().finish())
}

pub async fn patch_flight_service(
    service: web::Data<ApiService>,
    id: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> Result<HttpResponse, RestErrorResponse> {
    let id = id.into_inner();
    info!("patch_flight_service:  {:?}", id);

    let patch = body.into_inner();

    let update_fn = Arc::new(move |service: FlightService| {
        let mut value = serde_json::to_value(&service)
            .map_err(|e| commons::api::errors::MetaStoreError::Serialization(e.to_string()))?;
        json_patch::merge(&mut value, &patch);
        info!("patch_flight_service: value={:?}", value);
        let service = serde_json::from_value(value)
            .map_err(|e| commons::api::errors::MetaStoreError::Deserialization(e.to_string()))?;
        Ok(service)
    });

    service.meta_store.update_flight_service(&id, update_fn).await?;

    audit_data_connection_types(service.as_ref()).await?;

    Ok(HttpResponse::NoContent().finish())
}

pub async fn check_existent_connection(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    id: web::Path<String>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("check_existent_connection: for tenant {:?}", ctx.tenant_id);

    let connection_id = id.into_inner();
    let tenant_id = ctx.tenant_id.clone();

    audit_data_connection(service.as_ref(), tenant_id.as_str(), connection_id.as_str()).await?;

    info!("Connection checked successfully");
    Ok(HttpResponse::NoContent().finish())
}

pub async fn test_credentials(
    service: web::Data<ApiService>,
    ctx: web::ReqData<ApiContext>,
    body: web::Json<TestCredentials>,
) -> Result<HttpResponse, RestErrorResponse> {
    info!("test_credentials: for tenant {:?}", ctx.tenant_id);

    let dct = service
        .meta_store
        .get_data_connection_type(ctx.tenant_id.as_str(), &body.data_connection_type_id)
        .await?;

    let flight = service
        .meta_store
        .get_flight_service_by_connector(&dct.resource.provider)
        .await?;

    service
        .flight_client(flight.resource.internal_url.as_str())
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

async fn flight_service_by_connection(
    service: &ApiService,
    tenant_id: &str,
    data_connection_id: &str,
) -> Result<FlightServiceResource, MetaStoreError> {
    let dc = service
        .meta_store
        .get_data_connection(tenant_id, data_connection_id)
        .await?;

    let dct = service
        .meta_store
        .get_data_connection_type(tenant_id, &dc.resource.data_connection_type_id)
        .await?;

    service
        .meta_store
        .get_flight_service_by_connector(&dct.resource.provider)
        .await
}

// ------------------------------------------------------------
// Tests
// ------------------------------------------------------------

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
    use commons::api::storage::MetaStoreReader;
    use commons::api::storage::SecretStore;
    use std::collections::HashMap;
    use std::sync::RwLock;

    use super::*;
    use crate::rest::API_VERSION;
    use crate::rest::errors::{json_config, query_config};
    use crate::rest::middleware::{trace_request, validate_headers};

    fn api_path(path: &str) -> String {
        format!("/api/{API_VERSION}/data{path}")
    }

    struct StubMetaStore;

    #[async_trait::async_trait]
    impl MetaStoreReader for StubMetaStore {
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
            tenant_id: &str,
            uid: &str,
        ) -> Result<DataConnectionTypeResource, commons::api::errors::MetaStoreError> {
            if tenant_id == "test-tenant" && matches!(uid, "ct-1" | "ct-disabled") {
                let (name, provider) = if uid == "ct-disabled" {
                    ("SQLite", "sqlite")
                } else {
                    ("PostgreSQL", "postgres")
                };
                Ok(DataConnectionTypeResource {
                    metadata: commons::api::ResourceMetadata {
                        id: uid.to_string(),
                        tenant_id: Some("test-tenant".to_string()),
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        updated_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    resource: DataConnectionType {
                        name: name.to_string(),
                        provider: provider.to_string(),
                        description: Some(format!("{name} database connection")),
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
    }

    #[async_trait::async_trait]
    impl commons::api::storage::FlightDiscoveryStore for StubMetaStore {
        async fn create_flight_service(
            &self,
            flight_service: &commons::api::flight_discovery::FlightService,
        ) -> Result<commons::api::flight_discovery::FlightServiceResource, commons::api::errors::MetaStoreError>
        {
            Ok(commons::api::flight_discovery::FlightServiceResource {
                metadata: commons::api::ResourceMetadata {
                    id: "fs-1".to_string(),
                    tenant_id: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                },
                resource: flight_service.clone(),
            })
        }
        async fn get_all_flight_services(
            &self,
        ) -> Result<
            commons::api::ResourceList<commons::api::flight_discovery::FlightServiceResource>,
            commons::api::errors::MetaStoreError,
        > {
            Ok(commons::api::ResourceList {
                total_count: 0,
                items: vec![],
            })
        }
        async fn get_flight_service_by_connector(
            &self,
            connector: &str,
        ) -> Result<commons::api::flight_discovery::FlightServiceResource, commons::api::errors::MetaStoreError>
        {
            Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                "flight service for connector '{connector}' not found"
            )))
        }
        async fn get_flight_service(
            &self,
            id: &str,
        ) -> Result<commons::api::flight_discovery::FlightServiceResource, commons::api::errors::MetaStoreError>
        {
            if id == "fs-1" {
                Ok(commons::api::flight_discovery::FlightServiceResource {
                    metadata: commons::api::ResourceMetadata {
                        id: "fs-1".to_string(),
                        tenant_id: None,
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        updated_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    resource: commons::api::flight_discovery::FlightService {
                        name: "flight-1".to_string(),
                        namespace: "test-ns".to_string(),
                        external_url: "http://flight:50051".to_string(),
                        internal_url: "http://flight:50051".to_string(),
                        supported_connectors: vec![],
                        status: Default::default(),
                    },
                })
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "flight service '{id}' not found"
                )))
            }
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
        async fn delete_flight_service(&self, id: &str) -> Result<(), commons::api::errors::MetaStoreError> {
            if id == "fs-1" {
                Ok(())
            } else {
                Err(commons::api::errors::MetaStoreError::ResourceNotFound(format!(
                    "flight service '{id}' not found"
                )))
            }
        }
    }

    #[async_trait::async_trait]
    impl MetaStore for StubMetaStore {
        async fn create_data_connection(
            &self,
            tenant_id: &str,
            data_connection: &DataConnection,
        ) -> Result<DataConnectionResource, commons::api::errors::MetaStoreError> {
            if !matches!(data_connection.data_connection_type_id.as_str(), "ct-1" | "ct-disabled") {
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

        async fn get_all_data_connection_types(
            &self,
        ) -> Result<ResourceList<DataConnectionTypeResource>, commons::api::errors::MetaStoreError> {
            Ok(ResourceList {
                total_count: 0,
                items: vec![],
            })
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
            None,
            None,
            "test-tenant".to_string(),
        ))
    }

    fn test_app_config(cfg: &mut web::ServiceConfig) {
        cfg.service(
            web::scope(&format!("/api/{API_VERSION}/data"))
                .wrap(middleware::from_fn(validate_headers))
                .wrap(middleware::from_fn(trace_request))
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
                .route("/connections/{id}/binary", web::get().to(get_binary_data))
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
    async fn test_create_connection_type_with_disabled_connector() {
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
                "name": "SQLite",
                "provider": "sqlite",
                "description": "Disabled SQLite connector",
                "credentials_fields": []
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["resource"]["provider"], "sqlite");
    }

    #[actix_web::test]
    async fn test_create_connection_with_disabled_connector() {
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
                "name": "my-disabled-connector",
                "data_connection_type_id": "ct-disabled",
                "format": "tabular",
                "credentials_ref": {
                    "secret": "my-disabled-connector-creds"
                },
                "properties": {}
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["resource"]["name"], "my-disabled-connector");
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
    async fn test_get_binary_data_missing_path() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(query_config())
                .configure(test_app_config),
        )
        .await;
        let req = test::TestRequest::get()
            .uri(&api_path("/connections/conn-1/binary"))
            .insert_header(("x-tenant-id", "test-tenant"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "invalid_query");
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

    fn discovery_path(path: &str) -> String {
        format!("/api/{API_VERSION}/discovery{path}")
    }

    fn test_discovery_app_config(cfg: &mut web::ServiceConfig) {
        cfg.service(
            web::scope(&format!("/api/{API_VERSION}/discovery"))
                .route("/flights", web::post().to(create_flight_service))
                .route("/flights/{id}", web::delete().to(delete_flight_service)),
        );
    }

    #[actix_web::test]
    async fn test_create_flight_service_unreachable() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_discovery_app_config),
        )
        .await;
        let req = test::TestRequest::post()
            .uri(&discovery_path("/flights"))
            .insert_header(("content-type", "application/json"))
            .set_json(serde_json::json!({
                "name": "flight-1",
                "namespace": "test-ns",
                "external_url": "http://127.0.0.1:1",
                "internal_url": "http://127.0.0.1:1",
                "status": { "ready": false }
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 503);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "connection");
    }

    #[actix_web::test]
    async fn test_create_flight_service_invalid_body() {
        let app = test::init_service(
            App::new()
                .app_data(test_service())
                .app_data(json_config())
                .configure(test_discovery_app_config),
        )
        .await;
        let req = test::TestRequest::post()
            .uri(&discovery_path("/flights"))
            .insert_header(("content-type", "application/json"))
            .set_payload("{}")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "invalid_json");
    }

    #[actix_web::test]
    async fn test_delete_flight_service() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_discovery_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&discovery_path("/flights/fs-1"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 204);
    }

    #[actix_web::test]
    async fn test_delete_flight_service_not_found() {
        let app = test::init_service(App::new().app_data(test_service()).configure(test_discovery_app_config)).await;
        let req = test::TestRequest::delete()
            .uri(&discovery_path("/flights/nonexistent"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "not_found");
    }
}
