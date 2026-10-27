use crate::flight::QueryContext;
use crate::flight::auth::auth_error_to_status;
use crate::flight::errors::{map_connector_error, map_meta_store_error};
use crate::flight::metrics;
use crate::flight::registry::ConnectorsRegistry;
use crate::flight::trace::TracedStream;

use arrow::datatypes::{DataType, Field, Schema};
use arrow_flight::{
    Action, ActionType, FlightDescriptor, FlightEndpoint, FlightInfo, Ticket,
    encode::FlightDataEncoderBuilder,
    error::FlightError,
    flight_service_server::FlightService,
    sql::{
        Command, CommandGetSqlInfo, CommandGetTables, CommandStatementQuery, ProstMessageExt, SqlInfo,
        TicketStatementQuery, metadata::SqlInfoDataBuilder, server::FlightSqlService,
    },
};
use commons::api::connection_types::DataConnectionTypeResource;
use commons::api::connections::{CredentialsRef, DataConnectionResource};
use commons::api::connector::BinaryQuery;
use commons::api::connector::{CredentialsResolver, FlightConnector, QueryOptions};
use commons::api::errors::ConnectorError;
use commons::api::storage::MetaStoreReader;
use commons::api::storage::{MetaStore, SecretStore};
use commons::api::{X_REMOTE_GROUPS, X_REMOTE_USER};
use futures::TryStreamExt;
use kube_utils::auth::{AuthError, AuthInfo, KubeAuthClient};
use prost::Message;
use prost::bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status};
use tracing::{debug, info};

const METHOD_GET_FLIGHT_INFO: &str = "arrow.flight.protocol.FlightService/GetFlightInfo";
const METHOD_DO_GET: &str = "arrow.flight.protocol.FlightService/DoGet";
const OPERATION_SQL_INFO: &str = "sql_info";
const OPERATION_STATEMENT: &str = "statement";
const OPERATION_BINARY: &str = "binary";
const STATUS_OK: &str = "OK";

const OPERATION_TABLES: &str = "tables";
const DOWNLOAD_TYPE_URL: &str = "dataconnethub.opendatahub.io/download";

fn grpc_status_label(status: &Status) -> &'static str {
    match status.code() {
        tonic::Code::Ok => "OK",
        tonic::Code::Cancelled => "Cancelled",
        tonic::Code::Unknown => "Unknown",
        tonic::Code::InvalidArgument => "InvalidArgument",
        tonic::Code::DeadlineExceeded => "DeadlineExceeded",
        tonic::Code::NotFound => "NotFound",
        tonic::Code::AlreadyExists => "AlreadyExists",
        tonic::Code::PermissionDenied => "PermissionDenied",
        tonic::Code::ResourceExhausted => "ResourceExhausted",
        tonic::Code::FailedPrecondition => "FailedPrecondition",
        tonic::Code::Aborted => "Aborted",
        tonic::Code::OutOfRange => "OutOfRange",
        tonic::Code::Unimplemented => "Unimplemented",
        tonic::Code::Internal => "Internal",
        tonic::Code::Unavailable => "Unavailable",
        tonic::Code::DataLoss => "DataLoss",
        tonic::Code::Unauthenticated => "Unauthenticated",
    }
}

pub struct DataIngestionService {
    pub(crate) connectors_registry: Arc<ConnectorsRegistry>,
    meta_store: Arc<dyn MetaStoreReader + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    sql_info: arrow_flight::sql::metadata::SqlInfoData,
    query_options: QueryOptions,
    auth_client: Option<Arc<dyn SecretAuthorizer>>,
}

/// SecretAuthorizer checks that a requester may read a connection's
/// credentials secret before the service resolves it on their behalf. It is
/// a trait so tests can substitute a stub for the API-server-backed client.
#[async_trait::async_trait]
pub trait SecretAuthorizer: Send + Sync {
    async fn authorize_secret(&self, auth_info: &AuthInfo, tenant_id: &str, secret_name: &str)
    -> Result<(), AuthError>;
}

#[async_trait::async_trait]
impl SecretAuthorizer for KubeAuthClient {
    async fn authorize_secret(
        &self,
        auth_info: &AuthInfo,
        tenant_id: &str,
        secret_name: &str,
    ) -> Result<(), AuthError> {
        KubeAuthClient::authorize_secret(self, auth_info, tenant_id, secret_name).await
    }
}

impl DataIngestionService {
    pub fn new(
        connectors_registry: Arc<ConnectorsRegistry>,
        meta_store: Arc<dyn MetaStore + Send + Sync>,
        secret_store: Arc<dyn SecretStore + Send + Sync>,
        query_options: QueryOptions,
    ) -> Self {
        let mut builder = SqlInfoDataBuilder::new();
        builder.append(SqlInfo::FlightSqlServerName, "Data Connect Hub");
        builder.append(SqlInfo::FlightSqlServerVersion, env!("CARGO_PKG_VERSION"));
        builder.append(SqlInfo::FlightSqlServerArrowVersion, "1.3");
        builder.append(SqlInfo::FlightSqlServerReadOnly, true);
        builder.append(SqlInfo::FlightSqlServerSql, true);
        builder.append(SqlInfo::FlightSqlServerSubstrait, false);

        Self {
            connectors_registry,
            meta_store,
            secret_store,
            sql_info: builder.build().expect("valid sql info"),
            query_options,
            auth_client: None,
        }
    }

    /// with_authorizer enables requester checks on connection credentials.
    /// When no authorizer is set the service resolves `credentials_ref`
    /// secrets without verifying the requester (development mode).
    pub fn with_authorizer(mut self, authorizer: Arc<dyn SecretAuthorizer>) -> Self {
        self.auth_client = Some(authorizer);
        self
    }

    /// check_secret_access verifies that the authenticated remote user may
    /// read the connection's credentials secret before the service resolves
    /// it on their behalf. It is a no-op when auth is disabled.
    pub(crate) async fn check_secret_access(
        &self,
        metadata: &MetadataMap,
        tenant_id: &str,
        connection: &DataConnectionResource,
    ) -> Result<(), Status> {
        let Some(auth) = self.auth_client.as_ref() else {
            return Ok(());
        };
        let auth_info = remote_auth_info(metadata)?;
        auth.authorize_secret(&auth_info, tenant_id, &connection.resource.credentials_ref.secret)
            .await
            .map_err(|e| auth_error_to_status(&e))?;
        Ok(())
    }

    fn query_to_string(query: &CommandGetSqlInfo) -> Vec<String> {
        query
            .info
            .iter()
            .map(|id| {
                SqlInfo::try_from(*id as i32)
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|_| format!("Unknown({id})"))
            })
            .collect()
    }

    pub(crate) async fn get_connector_by_type_id(
        &self,
        tenant_id: &str,
        data_connection_type_id: &str,
    ) -> Result<(DataConnectionTypeResource, &Arc<dyn FlightConnector>), Status> {
        let data_connection_type = self
            .meta_store
            .get_data_connection_type(tenant_id, data_connection_type_id)
            .await
            .map_err(map_meta_store_error)?;

        let connector = self
            .connectors_registry
            .get_connector(data_connection_type.resource.provider.as_str())
            .map_err(map_connector_error)?;

        Ok((data_connection_type, connector))
    }

    #[tracing::instrument(
        skip_all,
        fields(
        tenant.id = %tenant_id,
        connection.id = %data_connection_id,
        connector.provider = tracing::field::Empty,
        )
    )]
    pub(crate) async fn get_connector_by_connection_id(
        &self,
        tenant_id: &str,
        data_connection_id: &str,
    ) -> Result<(DataConnectionResource, &Arc<dyn FlightConnector>), Status> {
        let connection = self
            .meta_store
            .get_data_connection(tenant_id, data_connection_id)
            .await
            .map_err(map_meta_store_error)?;

        let data_connection_type = self
            .meta_store
            .get_data_connection_type(tenant_id, connection.resource.data_connection_type_id.as_str())
            .await
            .map_err(map_meta_store_error)?;

        let connector = self
            .connectors_registry
            .get_connector(data_connection_type.resource.provider.as_str())
            .map_err(map_connector_error)?;
        tracing::Span::current().record("connector.provider", &data_connection_type.resource.provider);

        Ok((connection, connector))
    }

    fn handle_get_flight_info_sql_info(
        &self,
        query: CommandGetSqlInfo,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let requested: Vec<String> = Self::query_to_string(&query);
        info!("get_flight_info_sql_info: {:?}", requested);

        let flight_descriptor = request.into_inner();
        let ticket = Ticket::new(query.as_any().encode_to_vec());
        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(self.sql_info.schema().as_ref())
            .map_err(|e| {
                tracing::error!(error = %e, "failed to encode sql_info schema");
                Status::internal("failed to encode sql_info schema")
            })?
            .with_descriptor(flight_descriptor)
            .with_endpoint(endpoint);

        Ok(Response::new(flight_info))
    }

    fn handle_do_get_sql_info(
        &self,
        query: CommandGetSqlInfo,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let requested: Vec<String> = Self::query_to_string(&query);
        info!("do_get_sql_info: {:?}", requested);

        let batch = query.into_builder(&self.sql_info).build().map_err(|e| {
            tracing::error!(error = %e, "failed to build sql_info response");
            Status::internal("failed to build response")
        })?;

        let stream = futures::stream::once(async { Ok(batch) });
        let flight_stream = FlightDataEncoderBuilder::new()
            .with_schema(self.sql_info.schema())
            .build(stream)
            .map_err(|e| {
                tracing::error!(error = %e, "failed to encode flight data");
                Status::internal("failed to encode sql_info response")
            });

        Ok(Response::new(
            Box::pin(TracedStream::new(OPERATION_SQL_INFO, flight_stream)) as <Self as FlightService>::DoGetStream,
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn handle_get_flight_info_tables(
        &self,
        query: CommandGetTables,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        debug!("get_flight_info_tables: include_schema={}", query.include_schema);

        let flight_descriptor = request.into_inner();
        let ticket = Ticket::new(query.as_any().encode_to_vec());
        let endpoint = FlightEndpoint::new().with_ticket(ticket);
        let schema = query.into_builder().schema();

        let flight_info = FlightInfo::new()
            .try_with_schema(&schema)
            .map_err(|e| {
                tracing::error!(error = %e, "failed to encode tables schema");
                Status::internal("failed to encode tables schema")
            })?
            .with_descriptor(flight_descriptor)
            .with_endpoint(endpoint)
            .with_total_records(-1);

        Ok(Response::new(flight_info))
    }

    #[tracing::instrument(skip_all)]
    async fn handle_do_get_tables(
        &self,
        query: CommandGetTables,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        debug!("do_get_tables: include_schema={}", query.include_schema);

        let metadata = request.metadata();

        let tenant_id = QueryContext::tenant_id(metadata)?;
        let connection_id = QueryContext::connection_id(metadata)?;

        let (connection, connector) = self.get_connector_by_connection_id(tenant_id, connection_id).await?;

        self.check_secret_access(metadata, tenant_id, &connection).await?;

        let reader = connector
            .get_reader(&connection, self as &dyn CredentialsResolver)
            .await
            .map_err(map_connector_error)?;

        let filter = query.table_name_filter_pattern.clone();
        let include_schema = query.include_schema;
        let tables = reader
            .list_tables(filter.as_deref(), include_schema)
            .await
            .map_err(map_connector_error)?;

        let mut builder = query.into_builder();
        for table in &tables {
            builder
                .append(
                    &table.catalog,
                    &table.schema_name,
                    &table.table_name,
                    &table.table_type,
                    &table.table_schema,
                )
                .map_err(|e| {
                    tracing::error!(error = %e, "failed to append table to builder");
                    Status::internal("failed to build tables response")
                })?;
        }

        let batch = builder.build().map_err(|e| {
            tracing::error!(error = %e, "failed to build tables response");
            Status::internal("failed to build tables response")
        })?;

        let schema = batch.schema();
        let stream = futures::stream::once(async { Ok(batch) });
        let flight_stream = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(stream)
            .map_err(|e| {
                tracing::error!(error = %e, "failed to encode flight data");
                Status::internal("failed to encode tables response")
            });

        Ok(Response::new(
            Box::pin(TracedStream::new(OPERATION_TABLES, flight_stream)) as <Self as FlightService>::DoGetStream,
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn handle_get_flight_info_statement(
        &self,
        query: CommandStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        debug!("Received SQL Query: '{}'", query.query);

        let metadata = request.metadata();
        let tenant_id = QueryContext::tenant_id(metadata)?;
        let connection_id = QueryContext::connection_id(metadata)?;
        let (connection, connector) = self.get_connector_by_connection_id(tenant_id, connection_id).await?;

        self.check_secret_access(metadata, tenant_id, &connection).await?;

        let reader = connector
            .get_reader(&connection, self as &dyn CredentialsResolver)
            .await
            .map_err(map_connector_error)?;

        let pg_state = reader.schema(query.query.as_str()).await.map_err(map_connector_error)?;

        let schema = pg_state.schema.clone();

        let ticket_stmt = TicketStatementQuery {
            statement_handle: Bytes::from(query.query),
        };
        let ticket = Ticket::new(ticket_stmt.as_any().encode_to_vec());

        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(&schema)
            .map_err(|e| {
                tracing::error!(error = %e, "failed to encode statement schema");
                Status::internal("failed to encode statement schema")
            })?
            .with_endpoint(endpoint)
            .with_total_records(-1)
            .with_total_bytes(-1);

        Ok(Response::new(flight_info))
    }

    #[tracing::instrument(skip_all)]
    async fn handle_do_get_statement(
        &self,
        ticket: TicketStatementQuery,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let query = String::from_utf8(ticket.statement_handle.to_vec())
            .map_err(|_| Status::invalid_argument("Invalid statement handle"))?;
        debug!("Retrieving data with SQL query: '{}'", query);

        let metadata = request.metadata();

        let tenant_id = QueryContext::tenant_id(metadata)?;
        let connection_id = QueryContext::connection_id(metadata)?;
        let (connection, connector) = self.get_connector_by_connection_id(tenant_id, connection_id).await?;

        self.check_secret_access(metadata, tenant_id, &connection).await?;

        let reader = connector
            .get_reader(&connection, self as &dyn CredentialsResolver)
            .await
            .map_err(map_connector_error)?;

        let state = reader.schema(query.as_str()).await.map_err(map_connector_error)?;

        let schema = state.schema.clone();

        let stream = reader
            .read_tabular(state, &self.query_options)
            .await
            .map_err(map_connector_error)?;

        let flight_stream = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(stream.map_err(|e| FlightError::ExternalError(Box::new(e))))
            .map_err(|e| match e {
                FlightError::ExternalError(inner) => match inner.downcast::<ConnectorError>() {
                    Ok(ce) => map_connector_error(*ce),
                    Err(other) => {
                        tracing::error!(error = %other, "unexpected error during streaming");
                        Status::internal("data source read failed")
                    },
                },
                other => {
                    tracing::error!(error = %other, "failed to encode flight data");
                    Status::internal("failed to encode statement response")
                },
            });

        Ok(Response::new(
            Box::pin(TracedStream::new(OPERATION_STATEMENT, flight_stream)) as <Self as FlightService>::DoGetStream,
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn handle_get_flight_info_fallback(
        &self,
        cmd: Command,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        info!("get_flight_info_fallback: {:?}", cmd);
        let Command::Unknown(any) = cmd else {
            return Err(Status::invalid_argument("expected custom command"));
        };

        if any.type_url != DOWNLOAD_TYPE_URL {
            return Err(Status::unimplemented(format!("unknown command type: {}", any.type_url)));
        }

        let metadata = request.metadata();

        let tenant_id = QueryContext::tenant_id(metadata)?.to_string();
        let connection_id = QueryContext::connection_id(metadata)?.to_string();

        let path =
            String::from_utf8(any.value.to_vec()).map_err(|_| Status::invalid_argument("Invalid download path"))?;

        let (connection, connector) = self.get_connector_by_connection_id(&tenant_id, &connection_id).await?;

        self.check_secret_access(metadata, &tenant_id, &connection).await?;

        let reader = connector
            .get_reader(&connection, self as &dyn CredentialsResolver)
            .await
            .map_err(map_connector_error)?;

        reader
            .can_read_binary(Arc::new(BinaryQuery::new(path)))
            .await
            .map_err(map_connector_error)?;

        let ticket = Ticket::new(any.encode_to_vec());
        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let schema = Arc::new(Schema::new(vec![Field::new("data", DataType::Binary, false)]));

        let flight_info = FlightInfo::new()
            .try_with_schema(&schema)
            .map_err(|e| {
                tracing::error!(error = %e, "failed to encode schema");
                Status::internal("failed to encode schema")
            })?
            .with_endpoint(endpoint)
            .with_descriptor(request.into_inner())
            .with_total_records(-1)
            .with_total_bytes(-1);

        Ok(Response::new(flight_info))
    }

    #[tracing::instrument(skip_all)]
    async fn handle_do_get_fallback(
        &self,
        request: Request<Ticket>,
        message: arrow_flight::sql::Any,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        info!("do_get_fallback: {:?}", message);

        let metadata = request.metadata();

        let tenant_id = QueryContext::tenant_id(metadata)?.to_string();
        let connection_id = QueryContext::connection_id(metadata)?.to_string();

        let path =
            String::from_utf8(message.value.to_vec()).map_err(|_| Status::invalid_argument("Invalid download path"))?;

        let (connection, connector) = self.get_connector_by_connection_id(&tenant_id, &connection_id).await?;

        self.check_secret_access(metadata, &tenant_id, &connection).await?;

        let reader = connector
            .get_reader(&connection, self as &dyn CredentialsResolver)
            .await
            .map_err(map_connector_error)?;

        let stream = reader
            .read_binary(Arc::new(BinaryQuery::new(path)))
            .await
            .map_err(map_connector_error)?;

        let schema = Arc::new(Schema::new(vec![Field::new("data", DataType::Binary, false)]));

        let flight_stream = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(stream.map_err(|e| FlightError::ExternalError(Box::new(e))))
            .map_err(|e| match e {
                FlightError::ExternalError(inner) => match inner.downcast::<ConnectorError>() {
                    Ok(ce) => map_connector_error(*ce),
                    Err(other) => {
                        tracing::error!(error = %other, "unexpected error during streaming");
                        Status::internal("data source read failed")
                    },
                },
                other => {
                    tracing::error!(error = %other, "failed to encode flight data");
                    Status::internal("failed to encode statement response")
                },
            });

        Ok(Response::new(
            Box::pin(TracedStream::new(OPERATION_BINARY, flight_stream)) as <Self as FlightService>::DoGetStream,
        ))
    }
}

#[tonic::async_trait]
impl FlightSqlService for DataIngestionService {
    type FlightService = DataIngestionService;

    async fn register_sql_info(&self, _id: i32, _result: &SqlInfo) {}

    async fn list_custom_actions(&self) -> Option<Vec<Result<ActionType, Status>>> {
        Some(Self::custom_actions())
    }

    async fn do_action_fallback(
        &self,
        request: Request<Action>,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        self.dispatch_action(request).await
    }

    async fn get_flight_info_sql_info(
        &self,
        query: CommandGetSqlInfo,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_GET_FLIGHT_INFO, OPERATION_SQL_INFO);
        let started = Instant::now();
        let result = self.handle_get_flight_info_sql_info(query, request);
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };
        metrics::observe_rpc(METHOD_GET_FLIGHT_INFO, OPERATION_SQL_INFO, status, started.elapsed());
        result
    }

    async fn do_get_sql_info(
        &self,
        query: CommandGetSqlInfo,
        _request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_DO_GET, OPERATION_SQL_INFO);
        let started = Instant::now();
        let result = self.handle_do_get_sql_info(query);
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };
        metrics::observe_rpc(METHOD_DO_GET, OPERATION_SQL_INFO, status, started.elapsed());
        result
    }

    async fn get_flight_info_tables(
        &self,
        query: CommandGetTables,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_GET_FLIGHT_INFO, OPERATION_TABLES);
        let started = Instant::now();
        let result = self.handle_get_flight_info_tables(query, request).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };
        metrics::observe_rpc(METHOD_GET_FLIGHT_INFO, OPERATION_TABLES, status, started.elapsed());
        result
    }

    async fn do_get_tables(
        &self,
        query: CommandGetTables,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_DO_GET, OPERATION_TABLES);
        let started = Instant::now();
        let result = self.handle_do_get_tables(query, request).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };
        metrics::observe_rpc(METHOD_DO_GET, OPERATION_TABLES, status, started.elapsed());
        result
    }

    async fn get_flight_info_statement(
        &self,
        query: CommandStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_GET_FLIGHT_INFO, OPERATION_STATEMENT);
        let started = Instant::now();
        let result = self.handle_get_flight_info_statement(query, request).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };
        metrics::observe_rpc(METHOD_GET_FLIGHT_INFO, OPERATION_STATEMENT, status, started.elapsed());
        result
    }

    async fn do_get_statement(
        &self,
        ticket: TicketStatementQuery,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_DO_GET, OPERATION_STATEMENT);
        let started = Instant::now();
        let result = self.handle_do_get_statement(ticket, request).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };

        metrics::observe_rpc(METHOD_DO_GET, OPERATION_STATEMENT, status, started.elapsed());
        result
    }

    async fn get_flight_info_fallback(
        &self,
        cmd: Command,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_GET_FLIGHT_INFO, OPERATION_BINARY);
        let started = Instant::now();
        let result = self.handle_get_flight_info_fallback(cmd, request).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };

        metrics::observe_rpc(METHOD_GET_FLIGHT_INFO, OPERATION_BINARY, status, started.elapsed());
        result
    }

    async fn do_get_fallback(
        &self,
        request: Request<Ticket>,
        message: arrow_flight::sql::Any,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let _guard = metrics::InFlightGuard::new(METHOD_DO_GET, OPERATION_BINARY);
        let started = Instant::now();
        let result = self.handle_do_get_fallback(request, message).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };

        metrics::observe_rpc(METHOD_DO_GET, OPERATION_BINARY, status, started.elapsed());
        result
    }
}

#[async_trait::async_trait]
impl CredentialsResolver for DataIngestionService {
    #[tracing::instrument(
        skip_all,
        fields(
        connection.id = %connection.metadata.id,
        secret.name = %connection.resource.credentials_ref.secret,
        )
    )]
    async fn resolve(&self, connection: &DataConnectionResource) -> Result<HashMap<String, String>, ConnectorError> {
        match (&connection.metadata.tenant_id, &connection.resource.credentials_ref) {
            (Some(tenant_id), CredentialsRef { secret }) => {
                let secret = self
                    .secret_store
                    .get_secret(tenant_id, secret)
                    .await
                    .map_err(|e| ConnectorError::ConnectionError(e.to_string()))?;
                Ok(secret.properties.clone())
            },
            _ => Err(ConnectorError::ConnectionError("No credentials found".to_string())),
        }
    }
}

/// remote_auth_info rebuilds the requester identity recorded by the auth
/// middleware from the x-remote-user and x-remote-groups headers.
fn remote_auth_info(metadata: &MetadataMap) -> Result<AuthInfo, Status> {
    let username = metadata
        .get(X_REMOTE_USER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Status::permission_denied("no authenticated user on request"))?;
    let groups = metadata
        .get(X_REMOTE_GROUPS)
        .and_then(|value| value.to_str().ok())
        .map(|joined| {
            joined
                .split(',')
                .filter(|group| !group.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(AuthInfo {
        username: username.to_string(),
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::remote_auth_info;
    use tonic::metadata::MetadataMap;

    fn metadata(pairs: &[(&str, &str)]) -> MetadataMap {
        let mut map = MetadataMap::new();
        for (key, value) in pairs {
            map.insert(
                tonic::metadata::MetadataKey::from_bytes(key.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn remote_auth_info_reads_user_and_groups() {
        let map = metadata(&[
            ("x-remote-user", "system:serviceaccount:tenant-a:dch-test-user"),
            ("x-remote-groups", "system:serviceaccounts,tenant-a-users"),
        ]);

        let info = remote_auth_info(&map).expect("auth info");
        assert_eq!(info.username, "system:serviceaccount:tenant-a:dch-test-user");
        assert_eq!(
            info.groups,
            vec!["system:serviceaccounts".to_string(), "tenant-a-users".to_string()]
        );
    }

    #[test]
    fn remote_auth_info_without_groups_defaults_to_empty() {
        let map = metadata(&[("x-remote-user", "system:serviceaccount:tenant-a:dch-test-user")]);

        let info = remote_auth_info(&map).expect("auth info");
        assert!(info.groups.is_empty());
    }

    #[test]
    fn remote_auth_info_with_empty_groups_header_yields_no_groups() {
        let map = metadata(&[
            ("x-remote-user", "system:serviceaccount:tenant-a:dch-test-user"),
            ("x-remote-groups", ""),
        ]);

        let info = remote_auth_info(&map).expect("auth info");
        assert!(info.groups.is_empty());
    }

    #[test]
    fn remote_auth_info_without_user_is_denied() {
        let map = metadata(&[("x-remote-groups", "system:serviceaccounts")]);

        let err = remote_auth_info(&map).expect_err("missing user must be denied");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }
}
