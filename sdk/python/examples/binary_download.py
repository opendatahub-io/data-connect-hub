"""Download binary data from a binary-format connection via the REST API.

Usage:
    python examples/binary_download.py

Requires a running DCH gateway (default: localhost:8443) and an existing
connection with format="binary". Set environment variables to override defaults:
    DCH_HOST, DCH_TOKEN, DCH_TENANT_ID, DCH_CA_CERT, DCH_INSECURE,
    DCH_CONNECTION_ID, DCH_BINARY_PATH, DCH_OUTPUT_FILE

DCH_OUTPUT_FILE defaults to download.bin and is overwritten if it exists.
The SDK buffers the complete response in memory before writing it.
"""

import os
from pathlib import Path

from data_connect_hub import DataConnectClient

connection_id = os.getenv("DCH_CONNECTION_ID", "")
binary_path = os.getenv("DCH_BINARY_PATH", "")
if not connection_id or not binary_path:
    print("Set DCH_CONNECTION_ID and DCH_BINARY_PATH for a binary-format connection.")
    raise SystemExit(1)

output_file = Path(os.getenv("DCH_OUTPUT_FILE", "download.bin"))

with DataConnectClient(
    endpoint=os.getenv("DCH_HOST", "localhost:8443"),
    token=os.getenv("DCH_TOKEN", ""),
    tenant_id=os.getenv("DCH_TENANT_ID", "opendatahub"),
    ca_cert=os.getenv("DCH_CA_CERT") or None,
    insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
) as client:
    data = client.download_binary(connection_id, binary_path)

output_file.write_bytes(data)
print(f"Wrote {len(data)} bytes to {output_file}")
