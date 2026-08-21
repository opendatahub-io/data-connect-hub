"""Flight OCI tests: query CSV, Parquet, and JSONL artifacts from an OCI registry.

Requires OCI registry credentials in the env file and test data pushed as OCI artifacts.
Skips automatically if OCI is not configured.
"""

from __future__ import annotations

import pyarrow as pa
import pytest

from data_connect_hub import DataConnectClient


class TestFlightOci:
    def test_csv_query(
        self, dch_client: DataConnectClient, oci_flight_connection: str, oci_csv_query: str | None
    ) -> None:
        if not oci_csv_query:
            pytest.skip("DCH_OCI_CSV_QUERY not set")
        table = dch_client.read(oci_csv_query, oci_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert rows["id"] == [1, 2, 3]
        assert rows["category"] == ["factuality_csv", "reasoning_csv", "safety_csv"]

    def test_parquet_query(
        self, dch_client: DataConnectClient, oci_flight_connection: str, oci_parquet_query: str | None
    ) -> None:
        if not oci_parquet_query:
            pytest.skip("DCH_OCI_PARQUET_QUERY not set")
        table = dch_client.read(oci_parquet_query, oci_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert rows["id"] == [11, 12, 13]
        assert rows["category"] == ["factuality_parquet", "reasoning_parquet", "safety_parquet"]

    def test_jsonl_query(
        self, dch_client: DataConnectClient, oci_flight_connection: str, oci_jsonl_query: str | None
    ) -> None:
        if not oci_jsonl_query:
            pytest.skip("DCH_OCI_JSONL_QUERY not set")
        table = dch_client.read(oci_jsonl_query, oci_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert rows["id"] == [21, 22, 23]
        assert rows["category"] == ["factuality_jsonl", "reasoning_jsonl", "safety_jsonl"]

    def test_binary_query(
        self, dch_client: DataConnectClient, oci_flight_connection: str, oci_binary_query: str | None
    ) -> None:
        if not oci_binary_query:
            pytest.skip("DCH_OCI_BINARY_QUERY not set")
        table = dch_client.read(oci_binary_query, oci_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows >= 1
        assert "data" in table.column_names
        assert "offset" in table.column_names
