import adbc_driver_flightsql.dbapi as dbapi

uri = "grpc://127.0.0.1:50051"
headers = {
    "x-data-connection-id": "57f315b1-eba0-42a2-bffd-62c03a27a3ee",
    "x-tenant-id": "marius",
    "authorization": "Bearer abc123",
}

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
