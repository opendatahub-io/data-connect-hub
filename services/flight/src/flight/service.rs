use crate::flight::errors::{map_connector_error, map_meta_store_error, map_secret_store_error};
use crate::flight::metrics;
use crate::flight::registry::ConnectorsRegistry;
use arrow_flight::{
    Action, ActionType, FlightDescriptor, FlightEndpoint, FlightInfo, Ticket,
    encode::FlightDataEncoderBuilder,
    error::FlightError,
    flight_service_server::FlightService,
    sql::{
        CommandGetSqlInfo, CommandStatementQuery, ProstMessageExt, SqlInfo, TicketStatementQuery,
        metadata::SqlInfoDataBuilder, server::FlightSqlService,
    },
};
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::storage::{MetaStore, SecretStore};
use commons::api::tabular::QueryOptions;
use commons::api::{X_DATA_CONNECTION_ID, X_TENANT_ID};
use futures::TryStreamExt;
use prost::Message;
use prost::bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::{debug, info};

const METHOD_GET_FLIGHT_INFO: &str = "arrow.flight.protocol.FlightService/GetFlightInfo";
const METHOD_DO_GET: &str = "arrow.flight.protocol.FlightService/DoGet";
const OPERATION_SQL_INFO: &str = "sql_info";
const OPERATION_STATEMENT: &str = "statement";
const STATUS_OK: &str = "OK";

const ACTION_GET_SUPPORTED_CONNECTORS: &str = "GetSupportedConnectors";

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

pub struct TabularDataService {
    connectors_registry: Arc<ConnectorsRegistry>,
    meta_store: Arc<dyn MetaStore + Send + Sync>,
    secret_store: Arc<dyn SecretStore + Send + Sync>,
    sql_info: arrow_flight::sql::metadata::SqlInfoData,
    query_options: QueryOptions,
}

#[derive(Serialize, Deserialize)]
pub struct ConnectorInfo {
    name: String,
    description: String,
}

impl TabularDataService {
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

    async fn get_connection(&self, tenant_id: &str, connection_id: &str) -> Result<DataConnectionResource, Status> {
        tracing::info!(
            "get_connection: tenant_id: {}, connection_id: {}",
            tenant_id,
            connection_id
        );
        let mut r = self
            .meta_store
            .get_data_connection(tenant_id, connection_id)
            .await
            .map_err(map_meta_store_error)?;

        tracing::info!("Resolving credentials");
        if let Some(Admin::SecretRef { secret_ref: s }) = &r.resource.admin {
            let secret = self
                .secret_store
                .get_secret(tenant_id, s)
                .await
                .map_err(map_secret_store_error)?;
            r.resource.admin = Some(Admin::Secret {
                name: secret.name.clone(),
                secret: secret.properties.clone(),
            });
        }

        Ok(r)
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
            .map_err(|e| Status::internal(e.to_string()))?
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

        let batch = query
            .into_builder(&self.sql_info)
            .build()
            .map_err(|e| Status::internal(e.to_string()))?;

        let stream = futures::stream::once(async { Ok(batch) });
        let flight_stream = FlightDataEncoderBuilder::new()
            .with_schema(self.sql_info.schema())
            .build(stream)
            .map_err(|e| Status::internal(e.to_string()));

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
        let connection_id = metadata
            .get(X_DATA_CONNECTION_ID)
            .ok_or(Status::invalid_argument(format!(
                "{X_DATA_CONNECTION_ID} header is required"
            )))?
            .to_str()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let tenant_id = metadata
            .get(X_TENANT_ID)
            .ok_or(Status::invalid_argument(format!("{X_TENANT_ID} header is required")))?
            .to_str()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let connection = self.get_connection(tenant_id, connection_id).await?;

        let data_connection_type = self
            .meta_store
            .get_data_connection_type(tenant_id, connection.resource.data_connection_type_id.as_str())
            .await
            .map_err(map_meta_store_error)?;

        let connector = self
            .connectors_registry
            .get_connector(data_connection_type.resource.provider.as_str())
            .map_err(map_connector_error)?;

        let reader = connector.get_reader(&connection).await.map_err(map_connector_error)?;

        let pg_state = reader.schema(query.query.as_str()).await.map_err(map_connector_error)?;

        let schema = pg_state.schema.clone();

        let ticket_stmt = TicketStatementQuery {
            statement_handle: Bytes::from(query.query),
        };
        let ticket = Ticket::new(ticket_stmt.as_any().encode_to_vec());

        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(&schema)
            .map_err(|e| Status::internal(e.to_string()))?
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
        let connection_id = metadata
            .get(X_DATA_CONNECTION_ID)
            .ok_or(Status::invalid_argument(format!(
                "{X_DATA_CONNECTION_ID} header is required"
            )))?
            .to_str()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let tenant_id = metadata
            .get(X_TENANT_ID)
            .ok_or(Status::invalid_argument(format!("{X_TENANT_ID} header is required")))?
            .to_str()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let connection = self.get_connection(tenant_id, connection_id).await?;

        let data_connection_type = self
            .meta_store
            .get_data_connection_type(tenant_id, connection.resource.data_connection_type_id.as_str())
            .await
            .map_err(map_meta_store_error)?;

        let connector = self
            .connectors_registry
            .get_connector(data_connection_type.resource.provider.as_str())
            .map_err(map_connector_error)?;

        let reader = connector.get_reader(&connection).await.map_err(map_connector_error)?;

        let state = reader.schema(query.as_str()).await.map_err(map_connector_error)?;

        let schema = state.schema.clone();

        let stream = reader
            .read(state, &self.query_options)
            .await
            .map_err(map_connector_error)?;

        let flight_stream = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(stream.map_err(|e| FlightError::ExternalError(Box::new(e))))
            .map_err(|e| Status::internal(e.to_string()));

        Ok(Response::new(
            Box::pin(flight_stream) as <Self as FlightService>::DoGetStream
        ))
    }
}

#[tonic::async_trait]
impl FlightSqlService for TabularDataService {
    type FlightService = TabularDataService;

    async fn register_sql_info(&self, _id: i32, _result: &SqlInfo) {}

    async fn list_custom_actions(&self) -> Option<Vec<Result<ActionType, Status>>> {
        Some(vec![Ok(ActionType {
            r#type: ACTION_GET_SUPPORTED_CONNECTORS.into(),
            description: "Returns the list of supported data connectors".into(),
        })])
    }

    async fn do_action_fallback(
        &self,
        request: Request<Action>,
    ) -> Result<Response<<Self as FlightService>::DoActionStream>, Status> {
        let action = request.get_ref();
        match action.r#type.as_str() {
            ACTION_GET_SUPPORTED_CONNECTORS => {
                let providers = self
                    .connectors_registry
                    .get_supported_connectors()
                    .iter()
                    .map(|p| ConnectorInfo {
                        name: p.provider(),
                        description: p.description(),
                    })
                    .collect::<Vec<ConnectorInfo>>();
                let body = serde_json::to_vec(&providers).map_err(|e| Status::internal(e.to_string()))?;
                let result = arrow_flight::Result { body: body.into() };
                Ok(Response::new(
                    Box::pin(futures::stream::once(async { Ok(result) })) as <Self as FlightService>::DoActionStream
                ))
            },
            _ => Err(Status::invalid_argument(format!("Unknown action: {}", action.r#type))),
        }
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
}
