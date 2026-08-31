use crate::flight::QueryContext;
use crate::flight::errors::{map_connector_error, map_meta_store_error};
use crate::flight::metrics;
use crate::flight::registry::ConnectorsRegistry;

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
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::connector::BinaryQuery;
use commons::api::connector::{CredentialsResolver, FlightConnector, QueryOptions};
use commons::api::errors::ConnectorError;
use commons::api::storage::{MetaStore, SecretStore};
use futures::TryStreamExt;
use prost::Message;
use prost::bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::{debug, info};

const METHOD_GET_FLIGHT_INFO: &str = "arrow.flight.protocol.FlightService/GetFlightInfo";
const METHOD_DO_GET: &str = "arrow.flight.protocol.FlightService/DoGet";
const OPERATION_SQL_INFO: &str = "sql_info";
const OPERATION_STATEMENT: &str = "statement";
const OPERATION_STATEMENT_FALLBACK: &str = "statement_fallback";
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
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    sql_info: arrow_flight::sql::metadata::SqlInfoData,
    query_options: QueryOptions,
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
        }
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
            Box::pin(flight_stream) as <Self as FlightService>::DoGetStream
        ))
    }

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
            Box::pin(flight_stream) as <Self as FlightService>::DoGetStream
        ))
    }

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
            Box::pin(flight_stream) as <Self as FlightService>::DoGetStream
        ))
    }

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
            Box::pin(flight_stream) as <Self as FlightService>::DoGetStream
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
        let started = Instant::now();
        let result = self.handle_get_flight_info_fallback(cmd, request).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };

        metrics::observe_rpc(METHOD_DO_GET, OPERATION_STATEMENT, status, started.elapsed());
        result
    }

    async fn do_get_fallback(
        &self,
        request: Request<Ticket>,
        message: arrow_flight::sql::Any,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let started = Instant::now();
        let result = self.handle_do_get_fallback(request, message).await;
        let status = match &result {
            Ok(_) => STATUS_OK,
            Err(e) => grpc_status_label(e),
        };

        metrics::observe_rpc(METHOD_DO_GET, OPERATION_STATEMENT_FALLBACK, status, started.elapsed());
        result
    }
}

#[async_trait::async_trait]
impl CredentialsResolver for DataIngestionService {
    async fn resolve(&self, connection: &DataConnectionResource) -> Result<HashMap<String, String>, ConnectorError> {
        match (&connection.metadata.tenant_id, &connection.resource.admin) {
            (Some(tenant_id), Some(Admin::SecretRef { secret_ref })) => {
                let secret = self
                    .secret_store
                    .get_secret(tenant_id, secret_ref)
                    .await
                    .map_err(|e| ConnectorError::ConnectionError(e.to_string()))?;
                Ok(secret.properties.clone())
            },
            (_, Some(Admin::Secret { name: _, secret })) => Ok(secret.clone()),
            _ => Err(ConnectorError::ConnectionError("No credentials found".to_string())),
        }
    }
}
