# Data Connect Hub Python SDK

Python client library for the [Data Connect Hub](https://github.com/opendatahub-io/data-connect-hub) service.

## Installation

> **Note:** This package is not yet published to PyPI. Install from source for now.

```bash
# REST only (default)
pip install sdk/python

# REST + Flight SQL
pip install "sdk/python[flight]"
```

## Quick Start

The client takes a single gateway `endpoint` — a host or `host:port`, no scheme required — and derives both the REST (`https://`) and Flight SQL (`grpc+tls://`) URLs from it. Only TLS endpoints are supported; use `insecure=True` or `ca_cert=` to control certificate verification.

```python
from data_connect_hub import CredentialsRef, DataConnectClient

client = DataConnectClient(
    endpoint="dch.example.com:8443",
    token="<your-token>",  # or use token_provider= for auto-refresh
    tenant_id="my-tenant",
)

# Or use a token provider for automatic refresh on 401:
client = DataConnectClient(
    endpoint="dch.example.com:8443",
    token_provider=lambda: get_fresh_token(),  # your function; called once, cached, refreshed on 401
    tenant_id="my-tenant",
)

# List connections (REST)
connections = client.list_connections()

# Get a specific connection
conn = client.get_connection("conn-id")

# Create a connection
conn = client.create_connection(
    name="my-db",
    connection_type_id="dct-a1b2c3d4",
    data_format="tabular",  # DataFormat: "tabular" | "binary"
    credentials_ref=CredentialsRef(secret="secret/my-db"),
)

# Query data via Flight SQL
table = client.read("SELECT * FROM prompts", connection_id="conn-uuid")
df = table.to_pandas()
```

## API Reference

The REST API is the source of truth for every model below. See the [REST API reference](https://opendatahub-io.github.io/data-connect-hub/) for the full request/response schemas.

### Connection Types (REST)

Connection types describe a category of data source (e.g. PostgreSQL). They define the provider backend and the credential fields required to connect.

```python
client.list_connection_types() -> list[ConnectionType]
client.get_connection_type(type_id) -> ConnectionType
client.create_connection_type(name=..., provider=..., description=..., credentials_fields=...) -> ConnectionType
client.update_connection_type(type_id, name=..., provider=..., description=..., credentials_fields=...) -> ConnectionType
client.delete_connection_type(type_id) -> None
```

#### `ConnectionType`

| Field | Type | Description |
|---|---|---|
| `id` | `str` | Unique identifier |
| `name` | `str` | Display name |
| `provider` | `str` | Backend driver (e.g. `"postgres"`) |
| `description` | `str \| None` | Optional description |
| `tenant_id` | `str` | Owning namespace |
| `created_at` | `datetime \| None` | Creation timestamp |
| `updated_at` | `datetime \| None` | Last update timestamp |
| `credentials_fields` | `list[CredentialField]` | Credential fields required to connect |
| `status` | `ConnectionTypeStatus` | Transports the provider supports |

Pass `id` as the `type_id` argument to `get_connection_type`, `update_connection_type`, and `delete_connection_type` — and as `connection_type_id` to `create_connection`.

`status.capabilities` reports which transports the provider supports (`flight` and `rest`, both `bool`), so you can check before issuing a Flight SQL query:

```python
ct = client.get_connection_type("dct-a1b2c3d4")
if ct.status.capabilities.flight:
    table = client.read("SELECT * FROM prompts", connection_id=conn.id)
```

#### `CredentialField`

Describes a single input field in the connection credential form.

| Field | Type | Description |
|---|---|---|
| `name` | `str` | Field key (used as the secret key) |
| `label` | `str` | Human-readable label |
| `description` | `str \| None` | Optional help text |
| `required` | `bool` | Whether the field must be provided |
| `type` | `str` | Rendering hint for the form (see below) |
| `enum_values` | `list[EnumValue] \| None` | Allowed values when `type` is `"enum"` |
| `default_value` | `str \| None` | Optional default value |

`EnumValue` has two fields: `value` (the stored string) and `label` (the display string).

**`type` values:**

| Value | Meaning |
|---|---|
| `"string"` | Free-text single-line input |
| `"enum"` | One of `enum_values` |

`type` is a free-form string that only tells a client how to render the input — the server neither validates nor interprets it. Its one credential check is that every field with `required=True` is present in the submitted secret. Every connection type shipped in [`config/connection-types/`](../../config/connection-types/) uses `"string"`; your own may use any other value (e.g. `"password"` to hint that input should be masked), and clients that do not recognize it should treat it as `"string"`. The authoritative definition is the `Field` schema in the [REST API reference](https://opendatahub-io.github.io/data-connect-hub/).

### Connection Management (REST)

A connection pairs a connection type with the actual credentials (stored in a Kubernetes secret) and tracks the live status of the data source.

```python
client.list_connections() -> list[DataConnection]
client.get_connection(connection_id) -> DataConnection
client.create_connection(name=..., connection_type_id=..., data_format=..., credentials_ref=..., properties=...) -> DataConnection
client.update_connection(connection_id, name=..., connection_type_id=..., data_format=..., credentials_ref=...) -> DataConnection
client.delete_connection(connection_id) -> None
```

#### `DataConnection`

| Field | Type | Description |
|---|---|---|
| `id` | `str` | Unique identifier |
| `name` | `str` | Display name |
| `data_connection_type_id` | `str` | `id` of the associated `ConnectionType` |
| `format` | `"tabular" \| "binary"` | Data format of the source (see below) |
| `tenant_id` | `str` | Owning namespace |
| `created_at` | `datetime` | Creation timestamp |
| `updated_at` | `datetime` | Last update timestamp |
| `credentials_ref` | `CredentialsRef` | Credential secret reference |
| `properties` | `dict[str, str]` | Driver-specific properties (values masked in repr) |
| `status` | `DataConnectionStatus` | Live connection health |

Pass `id` as the `connection_id` argument to `get_connection`, `update_connection`, `delete_connection`, and the Flight SQL methods.

**`format` values:**

| Value | Meaning | Providers |
|---|---|---|
| `"tabular"` | Queried with SQL, returns rows | `postgres`, `sqlite`, `elasticsearch`, `milvus`, `neo4j`, `uri`, `s3` |
| `"binary"` | Opaque objects addressed by path | `s3`, `uri` |

Tabular connections are read with the [Flight SQL methods](#tabular-data-queries-flight-sql). Binary connections are managed through the same REST methods as tabular ones, but reading their contents uses a separate Flight download path that **this SDK does not wrap yet** — there is no `client.download(...)`. Until it is added, use `pyarrow.flight` directly; see [`hack/py-tools/samples/binary_download.py`](../../hack/py-tools/samples/binary_download.py).

You normally set `format` once, at `create_connection`, but it is not immutable: `update_connection(connection_id, data_format=...)` changes it, and the server accepts the new value without checking it against the provider or re-evaluating `status`. So switching a `postgres` connection to `binary` succeeds, leaves `status` reporting `ready`, and fails only when you try to read.

`credentials_ref` is a reference to a Kubernetes secret containing the connection credentials. Use `CredentialsRef(secret="secret-name")` where `secret-name` is the **name** of an existing secret in the tenant namespace (the namespace named by the connection's `tenant_id` that you passed to `DataConnectClient`). This is a bare secret name, not a `namespace/name` pair; cross-namespace references are not supported. If the secret is missing or unreadable, `status.state` becomes `"not_ready"`. The secret's keys must cover every `CredentialField` on the connection type that has `required=True`.

**`DataConnectionStatus`:**

| Field | Type | Description |
|---|---|---|
| `state` | `"ready" \| "ingestion_not_ready" \| "not_ready"` | Connection health (see below) |
| `message` | `str \| None` | Status detail message |
| `updated_at` | `datetime \| None` | When the status was last evaluated |

**`state` values:**

| Value | Meaning |
|---|---|
| `"ready"` | Credentials are valid and the source is queryable |
| `"ingestion_not_ready"` | Credentials are valid, but the source cannot be queried |
| `"not_ready"` | The referenced secret is missing or invalid |

### Tabular Data Queries (Flight SQL)

```python
client.read(sql, connection_id) -> pyarrow.Table          # full result as Arrow Table
client.read_pandas(sql, connection_id) -> pd.DataFrame    # full result as pandas DataFrame
client.read_batches(sql, connection_id) -> Generator[RecordBatch]  # stream of Arrow RecordBatches
client.get_tables(connection_id) -> pyarrow.Table         # table metadata
client.server_info() -> dict                              # server metadata
```

`read_batches` returns a generator that streams results instead of buffering the full result set in memory. The underlying cursor and connection are closed automatically when the generator is exhausted or garbage-collected:

```python
for batch in client.read_batches("SELECT * FROM prompts", "conn-uuid"):
    process(batch)
```

A server-side failure surfaced mid-stream raises `DCHQueryError`. Automatic token refresh applies when the stream is opened; an authentication failure that occurs after the stream is open is not retried.

These require the `flight` extra. On a REST-only install the client still imports and all REST calls work; the first Flight call raises `DCHConfigError` telling you to install `data-connect-hub[flight]`.

## Error Handling

Every failure raised by the SDK derives from `DCHError`, so a single `except` covers transport failures, HTTP errors, and malformed responses alike:

```python
from data_connect_hub import DCHError, DCHNotFoundError

try:
    conn = client.get_connection("conn-uuid")
except DCHNotFoundError:
    ...
except DCHError as exc:  # connection, timeout, auth, schema drift, ...
    ...
```

| Exception | Raised when |
|---|---|
| `DCHConfigError` | Invalid client configuration or argument (e.g. a blank id) |
| `DCHConnectionError` | The server was unreachable or the transport failed |
| `DCHTimeoutError` | The request exceeded `rest_timeout` |
| `DCHAuthenticationError` / `DCHForbiddenError` | HTTP 401 / 403 |
| `DCHNotFoundError` | HTTP 404 |
| `DCHValidationError` | HTTP 400 / 422 |
| `DCHServerError` | HTTP 5xx |
| `DCHResponseError` | The response was not JSON, or did not match the expected schema |
| `DCHQueryError` | A Flight SQL query failed |

Transient failures — HTTP 429/502/503/504, timeouts, and network or protocol errors — are retried automatically with exponential backoff on idempotent methods. See `max_retries`, `backoff_base`, and `backoff_max`.

## Requirements

- Python 3.11+
- Core dependencies: httpx, pydantic
- Flight SQL extras: adbc-driver-flightsql, pyarrow, pandas (`pip install "data-connect-hub[flight]"`)

## Releases

The package version is maintained in `src/data_connect_hub/_version.py`. Before creating a release tag, set it to the matching PEP 440 version, run `make sdk-install`, and then run `make sdk-package-check`. The release tag must be `v` followed by that same version, for example `v0.1.0`.

## Contributing

See [CONTRIBUTING.md](../../CONTRIBUTING.md) for development setup, commands, and contribution guidelines.
