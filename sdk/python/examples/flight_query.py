"""Query data via Flight SQL.

Usage:
    python examples/flight_query.py

Requires a running DCH gateway (default: localhost:8443).
Set environment variables to override defaults:
    DCH_HOST, DCH_TOKEN, DCH_TENANT_ID, DCH_CONNECTION_ID,
    DCH_CA_CERT, DCH_INSECURE
"""

import os

from data_connect_hub import DataConnectClient

client = DataConnectClient(
    endpoint=os.getenv("DCH_HOST", "localhost:8443"),
    token=os.getenv("DCH_TOKEN", ""),
    tenant_id=os.getenv("DCH_TENANT_ID", "opendatahub"),
    ca_cert=os.getenv("DCH_CA_CERT", None),
    insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
)

connection_id = os.getenv("DCH_CONNECTION_ID", "")
if not connection_id:
    print("Set DCH_CONNECTION_ID to a valid connection UUID.")
    raise SystemExit(1)

# Server metadata
info = client.server_info()
print("Server info:")
for key, value in info.items():
    print(f"  {key}: {value}")
print()

# Full query — returns a pyarrow.Table
table = client.read("SELECT * FROM test_prompts", connection_id)
print(table.to_pandas())
print()

# Full query — returns a pandas DataFrame directly
df = client.read_pandas("SELECT * FROM test_prompts", connection_id)
print(df)
