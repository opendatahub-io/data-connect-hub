"""Download binary data from a binary-format connection via the REST API.

Usage:
    python examples/binary_download.py

Requires a running DCH gateway (default: localhost:8443) and an existing
connection with format="binary". Set environment variables to override defaults:
    DCH_HOST, DCH_TOKEN, DCH_TENANT_ID, DCH_CA_CERT, DCH_INSECURE,
    DCH_CONNECTION_ID, DCH_BINARY_PATH, DCH_OUTPUT_FILE

DCH_OUTPUT_FILE defaults to download.bin and is atomically replaced after the
SDK finishes streaming the response to a temporary file.
"""

import os
from pathlib import Path
from tempfile import TemporaryDirectory

from data_connect_hub import DataConnectClient

connection_id = os.getenv("DCH_CONNECTION_ID", "")
binary_path = os.getenv("DCH_BINARY_PATH", "")
if not connection_id or not binary_path:
    print("Set DCH_CONNECTION_ID and DCH_BINARY_PATH for a binary-format connection.")
    raise SystemExit(1)

output_dir = Path.cwd().resolve()
configured_output = Path(os.getenv("DCH_OUTPUT_FILE", "download.bin"))
output_file = (output_dir / configured_output).resolve()
if output_file.parent != output_dir:
    raise ValueError("DCH_OUTPUT_FILE must name a file in the current directory")

with TemporaryDirectory(dir=output_file.parent) as temporary_directory:
    temporary_path = Path(temporary_directory) / output_file.name
    with (
        DataConnectClient(
            endpoint=os.getenv("DCH_HOST", "localhost:8443"),
            token=os.getenv("DCH_TOKEN", ""),
            tenant_id=os.getenv("DCH_TENANT_ID", "opendatahub"),
            ca_cert=os.getenv("DCH_CA_CERT") or None,
            insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
        ) as client,
        temporary_path.open("wb") as output,
    ):
        for chunk in client.download_binary(connection_id, binary_path):
            output.write(chunk)
    temporary_path.replace(output_file)

print(f"Wrote {output_file.stat().st_size} bytes to {output_file}")
