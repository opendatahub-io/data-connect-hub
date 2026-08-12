"""Manage connection types via the REST API.

Usage:
    python examples/connection_types.py

Requires a running DCH rest-service (default: http://localhost:8080).
Set environment variables to override defaults:
    DCH_REST_URL, DCH_TOKEN, DCH_TENANT_ID, DCH_CA_CERT, DCH_INSECURE
"""

import os

from data_connect_hub import DataConnectClient

client = DataConnectClient(
    rest_url=os.getenv("DCH_REST_URL", "http://localhost:8080"),
    token=os.getenv("DCH_TOKEN", ""),
    tenant_id=os.getenv("DCH_TENANT_ID", "default"),
    ca_cert=os.getenv("DCH_CA_CERT") or None,
    insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
)

# List connection types
types = client.list_connection_types()
print(f"Found {len(types)} connection type(s):")
for ct in types:
    print(f"  [{ct.id}] {ct.name}: {ct.description}")

# Create a custom connection type
new_type = client.create_connection_type(
    name="custom-mysql",
    provider="mysql",
    description="MySQL connector for analytics",
)
print(f"\nCreated type: {new_type.id} ({new_type.name})")

try:
    # Update it
    updated = client.update_connection_type(new_type.id, description="Updated description")
    print(f"Updated description: {updated.description}")
finally:
    # Clean up
    client.delete_connection_type(new_type.id)
    print(f"Deleted type {new_type.id}")
