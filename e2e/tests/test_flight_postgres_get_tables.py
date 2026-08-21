"""Flight SQL schema discovery: CommandGetTables (PostgreSQL).

Requires run-e2e.sh to have seeded dch_e2e.cities and set DCH_PG_SECRET.
"""

from __future__ import annotations

import pyarrow as pa
import pytest

from data_connect_hub import DataConnectClient


class TestFlightGetTables:
    """Test GetFlightInfo/DoGet for CommandGetTables requests."""

    def test_list_tables(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        """Tables are returned without schema by default."""
        table = dch_client.get_tables(pg_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows > 0
        assert set(table.column_names) >= {"catalog_name", "db_schema_name", "table_name", "table_type"}

        names = table.column("table_name").to_pylist()
        schemas = table.column("db_schema_name").to_pylist()
        print(f"\n[PG] Discovered {table.num_rows} tables:")
        for schema, name in zip(schemas, names, strict=True):
            print(f"  {schema}.{name}")

    def test_filter_by_name(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        """Filtering by table name returns only matching tables."""
        table = dch_client.get_tables(pg_flight_connection, table_name_filter="cities")
        assert isinstance(table, pa.Table)
        assert table.num_rows == 1
        names = table.column("table_name").to_pylist()
        assert "cities" in names

    def test_include_schema(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        """When include_schema=True, table_schema column contains valid IPC schema bytes."""
        table = dch_client.get_tables(pg_flight_connection, table_name_filter="cities", include_schema=True)
        assert isinstance(table, pa.Table)
        assert "table_schema" in table.column_names
        assert table.num_rows == 1

        schema_bytes = table.column("table_schema")[0].as_py()
        assert schema_bytes is not None and len(schema_bytes) > 0

        schema = pa.ipc.read_schema(pa.BufferReader(schema_bytes))
        field_names = [f.name for f in schema]
        print("\n[PG] Schema for 'cities':")
        for f in schema:
            print(f"  {f.name}: {f.type} (nullable={f.nullable})")

        assert "id" in field_names
        assert "name" in field_names
        assert "country" in field_names
        assert "population" in field_names

    def test_filter_no_match(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        """Filtering for a non-existent table returns zero rows."""
        table = dch_client.get_tables(pg_flight_connection, table_name_filter="nonexistent_table_xyz")
        assert isinstance(table, pa.Table)
        assert table.num_rows == 0

    def test_schema_reflects_types(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        """Returned schema accurately reflects column data types and nullability."""
        table = dch_client.get_tables(pg_flight_connection, table_name_filter="cities", include_schema=True)
        schema_bytes = table.column("table_schema")[0].as_py()
        schema = pa.ipc.read_schema(pa.BufferReader(schema_bytes))

        id_field = schema.field("id")
        assert pa.types.is_integer(id_field.type)

        name_field = schema.field("name")
        assert pa.types.is_string(name_field.type) or pa.types.is_large_string(name_field.type)
