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

The client takes a single gateway `endpoint` — a host or `host:port`, no scheme
required — and derives both the REST (`https://`) and Flight SQL
(`grpc+tls://`) URLs from it.
Only TLS endpoints are supported; use `insecure=True` or `ca_cert=` to control
certificate verification.

```python
from data_connect_hub import AdminSecretRef, DataConnectClient

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
    admin=AdminSecretRef(secret_ref="secret/my-db"),
)

# Query data via Flight SQL
table = client.read("SELECT * FROM prompts", connection_id="conn-uuid")
df = table.to_pandas()
```

## API Reference

### Connection Management (REST)

```python
client.list_connections() -> list[DataConnection]
client.get_connection(connection_id) -> DataConnection
client.create_connection(name=..., connection_type_id=..., data_format=..., admin=..., properties=...) -> DataConnection
client.update_connection(connection_id, name=..., connection_type_id=..., data_format=..., admin=...) -> DataConnection
client.delete_connection(connection_id) -> None
```

### Connection Types (REST)

```python
client.list_connection_types() -> list[ConnectionType]
client.get_connection_type(type_id) -> ConnectionType
client.create_connection_type(name=..., provider=..., description=..., credentials_fields=...) -> ConnectionType
client.update_connection_type(type_id, name=..., provider=..., description=..., credentials_fields=...) -> ConnectionType
client.delete_connection_type(type_id) -> None
```

### Tabular Data Queries (Flight SQL)

```python
client.read(sql, connection_id) -> pyarrow.Table       # full result as Arrow Table
client.read_pandas(sql, connection_id) -> pd.DataFrame # full result as pandas DataFrame
client.get_tables(connection_id) -> pyarrow.Table      # table metadata
client.server_info() -> dict                           # server metadata
```

These require the `flight` extra. On a REST-only install the client still
imports and all REST calls work; the first Flight call raises `DCHConfigError`
telling you to install `data-connect-hub[flight]`.

## Development

A virtual environment at `sdk/python/.venv` is created automatically on first run.
If `VIRTUAL_ENV` is already set (e.g. a manually activated venv), the Makefile uses the system Python directly.

```bash
make sdk-install     # install in editable mode with dev deps
make sdk-test        # run tests with coverage
make sdk-lint        # ruff check + format check
make sdk-fmt         # auto-format
make sdk-typecheck   # run mypy strict type checking
make sdk-all         # lint + typecheck + test
```

## Requirements

- Python 3.11+
- Core dependencies: httpx, pydantic
- Flight SQL extras: adbc-driver-flightsql, pyarrow, pandas (`pip install "data-connect-hub[flight]"`)
