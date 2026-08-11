"""Manage connections via the REST API.

Usage:
    python examples/connections.py

Requires a running DCH rest-service (default: http://localhost:8080).
Set environment variables to override defaults:
    DCH_REST_URL, DCH_TOKEN, DCH_TENANT_ID
"""

import os

from data_connect_hub import DataConnectClient

client = DataConnectClient(
    rest_url=os.getenv("DCH_REST_URL", "http://localhost:8080"),
    token=os.getenv("DCH_TOKEN", ""),
    tenant_id=os.getenv("DCH_TENANT_ID", "default"),
)

# List all connections
connections = client.list_connections()
print(f"Found {len(connections)} connection(s):")
for conn in connections:
    print(f"  [{conn.id}] {conn.name} ({conn.provider})")

# Create a new connection
new_conn = client.create_connection(
    name="example-postgres",
    namespace="default",
    provider="postgres",
    data_format="arrow",
    location_url="postgresql://localhost:5432/mydb",
)
print(f"\nCreated connection: {new_conn.id}")

try:
    # Fetch it back
    fetched = client.get_connection(new_conn.id)
    print(f"Fetched: {fetched.name} in namespace '{fetched.namespace}'")

    # Update it
    updated = client.update_connection(new_conn.id, name="renamed-postgres")
    print(f"Renamed to: {updated.name}")
finally:
    # Clean up
    client.delete_connection(new_conn.id)
    print(f"Deleted connection {new_conn.id}")
