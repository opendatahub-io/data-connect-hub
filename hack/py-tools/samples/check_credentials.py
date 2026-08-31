# CheckCredentials: validates credentials without creating a data connection.
#
# The action body is an Arrow IPC stream with a RecordBatch of key/value pairs.
# Required key: "data_connection_type_id" (the provider type, e.g. "postgres").
# Credential keys are prefixed with "secret." (e.g. "secret.HOST", "secret.PORT").

import pyarrow
import pyarrow.flight as flight

uri = "grpc://127.0.0.1:50051"
headers = {
    "x-tenant-id": "marius",
    "authorization": "Bearer abc123",
}

client = flight.connect(uri)
options = flight.FlightCallOptions(headers=[(k.encode(), v.encode()) for k, v in headers.items()])

keys = [
    "data_connection_type_id",
    "secret.URI",
]
values = [
    "d2f9e9e4-597a-4879-85d5-c4bbb6a05b63",
    "postgresql://mdanciu@localhost:5432/mdanciu",
]

batch = pyarrow.RecordBatch.from_pydict({
    "key": keys,
    "value": values,
})

sink = pyarrow.BufferOutputStream()
writer = pyarrow.ipc.new_stream(sink, batch.schema)
writer.write_batch(batch)
writer.close()
body = sink.getvalue().to_pybytes()

list(client.do_action(flight.Action("CheckCredentials", body), options))
print("Credentials checked successfully")
