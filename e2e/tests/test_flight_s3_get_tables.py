"""Flight SQL schema discovery: CommandGetTables (S3).

Requires run-e2e.sh to have seeded S3 test data and set DCH_S3_SECRET.
"""

from __future__ import annotations

import pyarrow as pa
import pytest

from data_connect_hub import DataConnectClient


class TestFlightS3GetTables:
    """Test GetFlightInfo/DoGet for CommandGetTables on S3 connections."""

    def test_list_tables(self, dch_client: DataConnectClient, s3_flight_connection: str) -> None:
        """All supported files in the bucket are listed."""
        table = dch_client.get_tables(s3_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows >= 3
        assert set(table.column_names) >= {"catalog_name", "db_schema_name", "table_name", "table_type"}

        names = table.column("table_name").to_pylist()
        print(f"\n[S3] Discovered {table.num_rows} tables:")
        for name in names:
            print(f"  {name}")

        assert any(n.endswith(".csv") for n in names)
        assert any(n.endswith(".parquet") for n in names)
        assert any(n.endswith(".jsonl") for n in names)

    def test_filter_by_name(self, dch_client: DataConnectClient, s3_flight_connection: str) -> None:
        """Filtering by exact file path returns only that file."""
        table = dch_client.get_tables(
            s3_flight_connection, table_name_filter="datasets/dch-test-prompts.parquet"
        )
        assert isinstance(table, pa.Table)
        assert table.num_rows == 1
        assert table.column("table_name")[0].as_py() == "datasets/dch-test-prompts.parquet"

    def test_filter_wildcard(self, dch_client: DataConnectClient, s3_flight_connection: str) -> None:
        """LIKE wildcard filter matches multiple files."""
        table = dch_client.get_tables(s3_flight_connection, table_name_filter="%dch-test-prompts%")
        assert isinstance(table, pa.Table)
        assert table.num_rows >= 3

    def test_include_schema(self, dch_client: DataConnectClient, s3_flight_connection: str) -> None:
        """When include_schema=True, table_schema contains valid IPC schema bytes."""
        table = dch_client.get_tables(
            s3_flight_connection,
            table_name_filter="datasets/dch-test-prompts.parquet",
            include_schema=True,
        )
        assert isinstance(table, pa.Table)
        assert "table_schema" in table.column_names
        assert table.num_rows == 1

        schema_bytes = table.column("table_schema")[0].as_py()
        assert schema_bytes is not None and len(schema_bytes) > 0

        schema = pa.ipc.read_schema(pa.BufferReader(schema_bytes))
        field_names = [f.name for f in schema]
        print("\n[S3] Schema for 'datasets/dch-test-prompts.parquet':")
        for f in schema:
            print(f"  {f.name}: {f.type} (nullable={f.nullable})")

        assert "id" in field_names
        assert "category" in field_names
        assert "prompt" in field_names

    def test_filter_no_match(self, dch_client: DataConnectClient, s3_flight_connection: str) -> None:
        """Filtering for a non-existent path returns zero rows."""
        table = dch_client.get_tables(s3_flight_connection, table_name_filter="nonexistent/file.parquet")
        assert isinstance(table, pa.Table)
        assert table.num_rows == 0
