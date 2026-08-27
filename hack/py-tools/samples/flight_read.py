import adbc_driver_flightsql.dbapi as dbapi
import pyarrow
import pyarrow.flight as flight

uri = "grpc://127.0.0.1:50051"
headers = {
    "x-data-connection-id": "57f315b1-eba0-42a2-bffd-62c03a27a3ee",
    "x-tenant-id": "marius",
    "authorization": "Bearer abc123",
}

client = flight.connect(uri)
options = flight.FlightCallOptions(headers=[(k.encode(), v.encode()) for k, v in headers.items()])

actions = list(client.list_actions(options))
print("Available actions:")
for action in actions:
    print(f"  {action.type}: {action.description}")
print()

results = list(client.do_action(flight.Action("GetSupportedConnectors", b""), options))
reader = pyarrow.ipc.open_stream(results[0].body.to_pybytes())
connectors = reader.read_all()
print(f"Supported connectors:\n{connectors.to_pandas()}")
print()

list(client.do_action(flight.Action("CheckConnection", b""), options))
print("Connection checked successfully")
print()

conn = dbapi.connect(
    uri,
    db_kwargs={
        f"adbc.flight.sql.rpc.call_header.{k}": v for k, v in headers.items()
    },
)

info = conn.adbc_get_info()
print("Server Info:")
for key, value in info.items():
    print(f"  {key}: {value}")
print()

cursor = conn.cursor()
cursor.execute("SELECT * FROM prompts")
table = cursor.fetch_arrow_table()
print(table.to_pandas())

cursor.close()
conn.close()

# Binary download via get_flight_info → do_get.
#
# The download path is encoded in a protobuf Any with a custom type URL.
# FlightSqlService decodes cmd as Any, sees the unknown type URL, and routes
# to get_flight_info_fallback which returns a ticket. do_get with that ticket
# routes to do_get_fallback which streams the binary data.
from google.protobuf.any_pb2 import Any as PbAny

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
