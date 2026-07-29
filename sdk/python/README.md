# Data Connect Hub Python SDK

Python client library for the [Data Connect Hub](https://github.com/opendatahub-io/data-connect-hub) service.

## Installation

> **Note:** This package is not yet published to PyPI. Install from source for now.

```bash
# From the repository root
pip install -e "sdk/python[dev]"
```

## Quick Start

```python
from data_connect_hub import DataConnectClient

client = DataConnectClient(
    rest_url="https://dch.example.com",
    token="<your-token>",  # raw token value, "Bearer" prefix added automatically
    tenant_id="my-tenant",
)

# List connections
connections = client.list_connections()

# Get a specific connection
conn = client.get_connection("conn-id")
```

## Async Usage

```python
async with DataConnectClient(
    rest_url="https://dch.example.com",
    token="<your-token>",
    tenant_id="my-tenant",
) as client:
    connections = await client.list_connections_async()
```

## API Reference

### Connection Management (REST)

```python
client.list_connections() -> list[DataConnection]
client.get_connection(connection_id) -> DataConnection
client.create_connection(name=..., namespace=..., provider=..., data_format=..., location_url=...) -> DataConnection
client.update_connection(connection_id, name=...) -> DataConnection
client.delete_connection(connection_id) -> None
```

### Connection Types (REST)

```python
client.list_connection_types() -> list[ConnectionType]
client.get_connection_type(type_id) -> ConnectionType
client.create_connection_type(name=..., description=...) -> ConnectionType
client.update_connection_type(type_id, name=...) -> ConnectionType
client.delete_connection_type(type_id) -> None
```

### Unstructured Data Ingestion (REST)

```python
client.ingest(connection_id) -> bytes
```

## Development

A virtual environment at `sdk/python/.venv` is created automatically on first run.
In CI (where `VIRTUAL_ENV` is already set by `actions/setup-python`), the system Python is used directly.

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
- Dependencies: httpx, pydantic
