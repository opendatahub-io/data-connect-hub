# DCH Audit Logging

## What is this?

Audit logging adds a record to the REST and Flight service logs for every
audited Data Connect Hub (DCH) API operation. Each record answers three
questions:

- **Who** made the request
- **What** they accessed (for example, connections, connection-types, or a
  data fetch)
- **Whether it succeeded or failed**

This is useful for security review, troubleshooting "who changed this
connection," and compliance.

## Is it on by default?

**No.** Audit logging is **off by default**. An administrator has to turn it
on for a given DCH instance.

## How to turn it on

Audit logging is configured on the `DataConnectService` custom resource (the
same resource that configures the rest of your DCH deployment). Add an
`audit` section with `enabled: true`:

```yaml
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
spec:
  audit:
    enabled: true
```

Apply it the same way you'd apply any other change to the CR, for example:

```console
oc apply -f dataconnectservice.yaml
```

To turn audit logging back off, set `enabled: false` (or remove the `audit`
section entirely — off is the default).

Changing this setting causes the REST and Flight service pods to restart, the
same way changing any other setting on the CR does.

## What an audit record looks like

Audit records show up as regular log lines from the `rest-server` and
`flight-server` containers, in the same JSON format as all other DCH logs,
but tagged `"logger":"audit"` so they're easy to pick out. Audit records omit
the standard `level` and `msg` fields; their result is represented by
`status_code` or `grpc_status` and, for failures, `error_code` and
`error_message`.

### Audit record for REST
Example:

```json
{
  "ts":"2026-11-03T20:06:59.894840Z",
  "logger":"audit",
  "service.name":"data-connect-hub",
  "request_id":"d2d56a23-baee-4cf0-b485-003ec20acb99",
  "actor":"system:serviceaccount:dch-tenant-a:dch-test-user",
  "request-path":"/api/v1alpha1/data/connection-types",
  "tenant_id":"dch-tenant-a",
  "resource":"connection_type",
  "action":"list",
  "status_code":200,
  "duration_ms":7
}
```

Example with errors:
```
{
  "ts":"2026-11-03T20:19:57.254948Z",
  "logger":"audit",
  "service.name":"data-connect-hub",
  "request_id":"f2dec0fa-4d86-446a-a816-d6e4559b4a7a",
  "actor":"system:serviceaccount:dch-tenant-a:dch-test-user",
  "request-path":"/api/v1alpha1/data/connections",
  "tenant_id":"dch-tenant-a",
  "resource":"connection",
  "action":"create",
  "status_code":422,
  "error_code":"unprocessable_entity",
  "error_message":"Unprocessable Entity",
  "duration_ms":5
}
```

### Audit record for Flight
- Example of direct Flight call:

```json
{
  "ts":"2026-11-03T21:08:53.786344Z",
  "logger":"audit",
  "service.name":"data-connect-hub",
  "request_id":"4297ad4e-5180-4adc-bd74-afbabb63ec3e",
  "actor":"system:serviceaccount:dch-tenant-a:dch-test-user",
  "caller_principal":"system:serviceaccount:dch-tenant-a:dch-test-user",
  "request-path":"/arrow.flight.protocol.FlightService/DoGet",
  "tenant_id":"dch-tenant-a",
  "resource":"connection",
  "action":"query",
  "resource_id":"339e4f76-be20-4df4-82c4-8e046e463f90",
  "grpc_status":"OK",
  "duration_ms":16
}
```

- Example of Flight call from REST. Here's the audit record from REST service:
```json
{
  "ts":"2026-11-03T21:40:47.103735Z",
  "logger":"audit",
  "service.name":"data-connect-hub",
  "request_id":"678fd57f-0435-46a9-bf8f-e1cb291c1a5a", <==
  "actor":"system:serviceaccount:dch-tenant-a:dch-test-user",
  "request-path":"/api/v1alpha1/data/test/credentials",
  "tenant_id":"dch-tenant-a",
  "resource":"credentials",
  "action":"test","status_code":502,
  "error_code":"connection_check_failed",
  "error_message":"Bad Gateway",
  "duration_ms":74
}
```

Here's the corresponding audit record from Flight service:

```json
{
  "ts":"2026-11-03T21:40:47.093497Z",
  "logger":"audit",
  "service.name":"data-connect-hub",
  "request_id":"eec9ee27-9f25-461d-935b-3fab474596ff",
  "parent_request_id":"678fd57f-0435-46a9-bf8f-e1cb291c1a5a",  <===
  "actor":"system:serviceaccount:dch-tenant-a:dch-test-user",  <===
  "caller_principal":"system:serviceaccount:dch-services:dch-rest-service-sa", <===
  "request-path":"/arrow.flight.protocol.FlightService/DoAction",
  "tenant_id":"dch-tenant-a",
  "resource":"credentials",
  "action":"check",
  "grpc_status":"InvalidArgument",
  "error_code":"InvalidArgument",
  "error_message":"gRPC request failed",
  "duration_ms":19
}
```

### Fields
Each field tells you something specific:

| Field | Meaning |
|---|---|
| `ts` | When the request happened (UTC, ISO 8601) |
| `actor` | Who is responsible for the request. REST uses `"anonymous"` when its proxy-set user header is missing. Flight uses `"unknown"` only before a principal can be authenticated. |
| `caller_principal` | Flight only: the bearer-token principal that was authenticated and authorized |
| `parent_request_id` | Flight only: the trusted REST request ID that caused an internal Flight call |
| `tenant_id` | Which tenant the request was scoped to |
| `resource` / `action` | What was accessed and what was done to it, e.g. `connection` / `create` |
| `resource_id` | The specific connection/resource affected, when applicable |
| `status_code` (REST) / `grpc_status` (Flight) | The technical result code |
| `error_code` / `error_message` | Present only when the request failed |
| `request_id` | A unique ID for this one request, useful for finding every log line related to it |
| `trace_id` / `span_id` | Links this audit record to the full distributed trace, when tracing is enabled |
| `duration_ms` | Handler duration. For Flight streams, this covers the full stream lifetime. For REST binary downloads, it covers response setup only. |

A single action taken through the REST API (for example, downloading data)
can trigger a follow-up call to the Flight service internally. When the call
comes from the configured REST service account, Flight trusts the forwarded
actor and valid UUID request ID. The Flight record then has a
`parent_request_id` matching the REST record's `request_id`, and its `actor`
shows the original REST user. `caller_principal` still identifies the REST
service account that directly called Flight.

Direct Flight callers use their authenticated bearer-token principal as both
`actor` and `caller_principal`. Flight ignores forwarded actor and request-ID
headers from every other principal, preventing a direct caller from spoofing
another user or linking itself to an unrelated REST request. If the trusted
REST service account does not forward an actor, Flight uses that service
account principal as the actor rather than `"unknown"`.

For REST binary downloads, `status_code` and `duration_ms` describe setting up
the streaming HTTP response. A REST `200` does not guarantee that the complete
body was transferred successfully. Use the corresponding Flight
`download_binary` record to determine the final transfer status and full
stream duration.

## What `resource` and `action` values you'll see

Every audit record's `resource` tells you *what kind of thing* was touched,
and `action` tells you *what was done to it*. Here's the full list:

| `resource` | What it is | `action` values you may see |
|---|---|---|
| `connection_type` | A type of data source DCH supports (e.g. Postgres, S3). REST only. | `list`, `create`, `get`, `update`, `delete` |
| `connection` | A configured connection to a data source. Shows up from **both** REST and Flight — see the breakdown just below the table, since the two layers log different actions against it. | `list`, `create`, `get`, `update`, `delete`, `check_readiness`, `download`, `query`, `list_tables`, `download_binary` |
| `connection_secret` | Exporting a connection's credentials to a Kubernetes Secret. REST only. | `export` |
| `credentials` | Credentials being tested or checked before they're saved. On the Flight service, this is `DoAction: CheckCredentials` (action `check`). | `test`, `check` |
| `flight_service` | A Flight backend service registered with DCH (its connection info, not the data it serves). REST only. | `create`, `update`, `delete` |
| `connector_catalog` | The list of connector types a Flight service supports. Comes from the Flight service's `DoAction: GetSupportedConnectors` (action `list`). | `list` |
| `flight_server` | The Flight service's own general capabilities, plus requests rejected before Flight can decode their operation. Generic `do_get`/`do_action` values appear only for malformed or unsupported payloads that cannot be mapped more specifically. | `get_sql_info`, `authenticate`, `do_get`, `do_action` |

### `connection`: REST vs. Flight actions

The `connection` resource is the one place where both services log against
the same resource name, so it's worth spelling out separately. The `actor`,
`tenant_id`, and `resource_id` (which connection) stay the same across both;
only the layer and `action` differ.

**On the REST service** — managing a connection's configuration:

- `list` — list all connections
- `create` — create a connection
- `get` — get a connection
- `update` — update a connection
- `delete` — delete a connection
- `check_readiness` — check a connection is reachable (internally calls Flight's `CheckDataConnection`, below)
- `download` — download a file through a connection (internally calls Flight's `do_get_fallback`, below)

**On the Flight service** — actually touching the connection's data:

- `list_tables` — `do_get_tables`: lists the tables available through the connection
- `query` — `do_get_statement`: runs a query and streams back the results — the main data-fetch path
- `download_binary` — `do_get_fallback`: streams a file through the connection (this is what REST's `download` triggers)
- `check_readiness` — `DoAction: CheckDataConnection`: checks the connection is reachable/healthy (this is what REST's `check_readiness` triggers)

When REST reaches the audited Flight operation, `check_readiness` and
`download` show up **twice** for one user action: once on the REST side and
once on the Flight side. If REST fails first while validating or looking up
the connection, credentials, connector, or Flight service, only the REST
record is produced. A download also produces only the REST record when its
unaudited `GetFlightInfo` negotiation fails before `DoGet` starts.

`list_tables` and `query`, on the other hand, are usually triggered directly
against the Flight service by a SQL client (JDBC/ODBC driver or BI tool), with
no REST call involved at all. Use `request_id`/`parent_request_id` (see above)
to tell a REST-triggered pair apart from a standalone Flight call.

REST health checks are not audited. Flight audit logging covers `DoGet` data
operations and `DoAction` executions, including authentication and
authorization failures on those paths. Flight health, `GetFlightInfo`,
`ListActions`, and all other Flight RPC methods are not audited. This includes
`Handshake`, `ListFlights`, `GetSchema`, `DoPut`, and `DoExchange`.

The `DoGet` data transfer or `DoAction` execution is the auditable operation
and produces the single record. `GetSupportedConnectors` and SQL-info `DoGet`
calls are audited because they access the connector catalog or Flight server
capabilities.

## Finding audit logs

If your cluster forwards logs to Loki (for example, via OpenShift Logging),
you can filter down to just audit records with a query like:

```logql
{kubernetes_namespace_name="dch-services"}
| json
| line_format `{{.message}}`
| json
| logger = "audit"
```

Add further filters the same way you would for any other DCH log — for
example, by `actor`, `resource`, or HTTP failure status:

```logql
{kubernetes_namespace_name="dch-services"}
| json
| line_format `{{.message}}`
| json
| logger = "audit"
| status_code >= 400
```

See the main [DCH Logging guide](loggings.md) for more background on the log
format and how logs reach Loki.

## Things to know

- Audit logging rides along on the same log stream as everything else — it
  doesn't send logs anywhere new by itself. Whether those logs end up
  long-term, tamper-proof storage depends on how your cluster's log
  forwarding and storage (e.g. LokiStack retention policy) are configured —
  that's a cluster administration concern, not something DCH controls.
