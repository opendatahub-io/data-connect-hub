"""Manage connections via the REST API.

Usage:
    python examples/connections.py

Requires a running DCH gateway (default: localhost:8443)
and an existing connection type (DCH_CONNECTION_TYPE_ID).
Set environment variables to override defaults:
    DCH_HOST, DCH_TOKEN, DCH_TENANT_ID, DCH_CA_CERT, DCH_INSECURE,
    DCH_CONNECTION_TYPE_ID, DCH_SECRET_NAME, DCH_CREDENTIALS_FILE,
    DCH_EXPORT_SECRET_NAME

When DCH_CREDENTIALS_FILE points to a JSON object, the example tests those
credentials and creates the connection with an inline Kubernetes secret.
Otherwise, DCH_SECRET_NAME must already exist in the tenant namespace.

DCH_EXPORT_SECRET_NAME is optional. It must differ from DCH_SECRET_NAME,
because export overwrites its target Kubernetes secret. Inline and exported
secrets are not deleted when the connection is deleted. For inline
credentials, this example generates a unique secret name unless
DCH_SECRET_NAME is set. An explicitly named inline secret must not already
exist. Delete it when finished:
    kubectl delete secret <secret-name> -n <tenant-namespace>
"""

import json
import os
import sys
from pathlib import Path
from uuid import uuid4

from data_connect_hub import CredentialsRef, DataConnectClient, InlineCredentials
from data_connect_hub.exceptions import DCHHTTPError

tenant_id = os.getenv("DCH_TENANT_ID", "opendatahub")
client = DataConnectClient(
    endpoint=os.getenv("DCH_HOST", "localhost:8443"),
    token=os.getenv("DCH_TOKEN", ""),
    tenant_id=tenant_id,
    ca_cert=os.getenv("DCH_CA_CERT") or None,
    insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
)

connection_type_id = os.getenv("DCH_CONNECTION_TYPE_ID", "")
if not connection_type_id:
    print("Set DCH_CONNECTION_TYPE_ID to a valid connection type UUID.")
    raise SystemExit(1)

credentials_file = os.getenv("DCH_CREDENTIALS_FILE", "")
credentials: InlineCredentials | None = None
credentials_ref: CredentialsRef | None = None

if credentials_file:
    secret_name = os.getenv("DCH_SECRET_NAME", f"example-credentials-{uuid4().hex[:8]}")
    try:
        credentials_data = json.loads(Path(credentials_file).read_text())
    except (OSError, json.JSONDecodeError) as exc:
        print(f"Unable to load DCH_CREDENTIALS_FILE: {exc}", file=sys.stderr)
        raise SystemExit(1) from None
    if not isinstance(credentials_data, dict) or not all(
        isinstance(key, str) and isinstance(value, str) for key, value in credentials_data.items()
    ):
        print("DCH_CREDENTIALS_FILE must contain a JSON object with string keys and values.", file=sys.stderr)
        raise SystemExit(1)

    # Validate credentials against the data source without persisting them.
    client.test_credentials(connection_type_id, credentials_data)
    print(f"Credentials are valid. Creating Kubernetes secret {secret_name}.")
    credentials = InlineCredentials(secret=secret_name, properties=credentials_data)
else:
    secret_name = os.getenv("DCH_SECRET_NAME", "my-db-creds")
    credentials_ref = CredentialsRef(secret=secret_name)

export_secret_name = os.getenv("DCH_EXPORT_SECRET_NAME", "")
if export_secret_name and export_secret_name == secret_name:
    print("DCH_EXPORT_SECRET_NAME must differ from DCH_SECRET_NAME.", file=sys.stderr)
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
        credentials_ref=credentials_ref,
        credentials=credentials,
    )
    print(f"\nCreated connection: {new_conn.id}")
except DCHHTTPError as exc:
    print(f"\nFailed to create connection: {exc}", file=sys.stderr)
    raise SystemExit(1) from None

try:
    # Re-check the saved credentials and data source, then fetch the new status.
    client.check_connection_readiness(new_conn.id)
    fetched = client.get_connection(new_conn.id)
    print(f"Fetched: {fetched.name} (tenant={fetched.tenant_id}, status={fetched.status.state})")

    if export_secret_name:
        client.export_connection(new_conn.id, export_secret_name)
        print(f"Exported connection to Kubernetes secret {export_secret_name}")
finally:
    # Clean up
    client.delete_connection(new_conn.id)
    print(f"Deleted connection {new_conn.id}")
    if credentials_file:
        print(f"Delete the inline credential secret when finished: {secret_name} (namespace {tenant_id})")
