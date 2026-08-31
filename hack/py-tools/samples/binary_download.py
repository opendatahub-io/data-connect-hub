# Binary download via get_flight_info -> do_get.
#
# The download path is encoded in a protobuf Any with a custom type URL.
# FlightSqlService decodes cmd as Any, sees the unknown type URL, and routes
# to get_flight_info_fallback which returns a ticket. do_get with that ticket
# routes to do_get_fallback which streams the binary data.

import pyarrow.flight as flight
from google.protobuf.any_pb2 import Any as PbAny

uri = "grpc://127.0.0.1:50051"
headers = {
    "x-data-connection-id": "57f315b1-eba0-42a2-bffd-62c03a27a3ee",
    "x-tenant-id": "marius",
    "authorization": "Bearer abc123",
}

client = flight.connect(uri)
options = flight.FlightCallOptions(headers=[(k.encode(), v.encode()) for k, v in headers.items()])

DOWNLOAD_TYPE_URL = "dataconnethub.opendatahub.io/download"
download_path = "some/file/path.parquet"

download_cmd = PbAny()
download_cmd.type_url = DOWNLOAD_TYPE_URL
download_cmd.value = download_path.encode("utf-8")
descriptor = flight.FlightDescriptor.for_command(download_cmd.SerializeToString())
info = client.get_flight_info(descriptor, options)
endpoint = info.endpoints[0]

print(f"Binary download: {download_path}")
reader = client.do_get(endpoint.ticket, options)
chunks = b""
for chunk in reader:
    for col in chunk.data.column("data"):
        chunks += col.as_py()
print(f"  result: {chunks.decode('utf-8')}")
