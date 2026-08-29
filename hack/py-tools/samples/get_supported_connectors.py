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

results = list(client.do_action(flight.Action("GetSupportedConnectors", b""), options))
reader = pyarrow.ipc.open_stream(results[0].body.to_pybytes())
connectors = reader.read_all()
print(f"Supported connectors:\n{connectors.to_pandas()}")
