"""Manage connection types via the REST API.

Usage:
    python examples/connection_types.py

Requires a running DCH rest-service (default: https://localhost:8443).
Set environment variables to override defaults:
    DCH_REST_URL, DCH_TOKEN, DCH_TENANT_ID, DCH_CA_CERT, DCH_INSECURE
"""

import os
import sys

from data_connect_hub import DataConnectClient
from data_connect_hub.exceptions import DCHHTTPError

client = DataConnectClient(
    rest_url=os.getenv("DCH_REST_URL", "https://localhost:8443"),
    token=os.getenv("DCH_TOKEN", ""),
    tenant_id=os.getenv("DCH_TENANT_ID", "opendatahub"),
    ca_cert=os.getenv("DCH_CA_CERT") or None,
    insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
)

# List connection types
types = client.list_connection_types()
print(f"Found {len(types)} connection type(s):")
for ct in types:
    print(f"  [{ct.id}] {ct.name}: {ct.description}")

# Create a connection type
try:
    new_type = client.create_connection_type(
        name="example-postgres",
        provider="postgres",
        description="PostgreSQL connector created by example",
    )
    print(f"\nCreated type: {new_type.id} ({new_type.name})")
except DCHHTTPError as exc:
    print(f"\nFailed to create connection type: {exc}", file=sys.stderr)
    raise SystemExit(1) from None

try:
    # Fetch it back
    fetched = client.get_connection_type(new_type.id)
    print(f"Fetched: {fetched.name} — {fetched.description}")
finally:
    # Clean up
    client.delete_connection_type(new_type.id)
    print(f"Deleted type {new_type.id}")
