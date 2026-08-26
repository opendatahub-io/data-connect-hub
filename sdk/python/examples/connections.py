"""Manage connections via the REST API.

Usage:
    python examples/connections.py

Requires a running DCH gateway (default: localhost:8443)
and an existing connection type (DCH_CONNECTION_TYPE_ID).
Set environment variables to override defaults:
    DCH_HOST, DCH_TOKEN, DCH_TENANT_ID, DCH_CA_CERT, DCH_INSECURE,
    DCH_CONNECTION_TYPE_ID
"""

import os
import sys

from data_connect_hub import AdminSecretRef, DataConnectClient
from data_connect_hub.exceptions import DCHHTTPError

client = DataConnectClient(
    endpoint=os.getenv("DCH_HOST", "localhost:8443"),
    token=os.getenv("DCH_TOKEN", ""),
    tenant_id=os.getenv("DCH_TENANT_ID", "opendatahub"),
    ca_cert=os.getenv("DCH_CA_CERT") or None,
    insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
)

connection_type_id = os.getenv("DCH_CONNECTION_TYPE_ID", "")
if not connection_type_id:
    print("Set DCH_CONNECTION_TYPE_ID to a valid connection type UUID.")
    raise SystemExit(1)

# List all connections
connections = client.list_connections()
print(f"Found {len(connections)} connection(s):")
for conn in connections:
    print(f"  [{conn.id}] {conn.name} (type={conn.data_connection_type_id}, format={conn.format})")

# Create a new connection
try:
    new_conn = client.create_connection(
        name="example-postgres",
        connection_type_id=connection_type_id,
        data_format="tabular",
        admin=AdminSecretRef(secret_ref="my-db-creds"),
        properties={"host": "localhost", "port": "5432", "dbname": "mydb"},
    )
    print(f"\nCreated connection: {new_conn.id}")
except DCHHTTPError as exc:
    print(f"\nFailed to create connection: {exc}", file=sys.stderr)
    raise SystemExit(1) from None

try:
    # Fetch it back
    fetched = client.get_connection(new_conn.id)
    print(f"Fetched: {fetched.name} (tenant={fetched.tenant_id})")
finally:
    # Clean up
    client.delete_connection(new_conn.id)
    print(f"Deleted connection {new_conn.id}")
