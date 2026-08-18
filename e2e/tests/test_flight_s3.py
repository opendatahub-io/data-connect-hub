"""Flight S3 tests: query CSV, Parquet, and JSONL files from S3.

Requires AWS credentials in the env file and test data on S3.
Skips automatically if S3 is not configured.
"""

from __future__ import annotations

import pyarrow as pa
import pytest

from data_connect_hub import DataConnectClient


class TestFlightS3:
    def test_csv_query(
        self, dch_client: DataConnectClient, s3_flight_connection: str, s3_csv_query: str | None
    ) -> None:
        if not s3_csv_query:
            pytest.skip("DCH_S3_CSV_QUERY not set")
        table = dch_client.read(s3_csv_query, s3_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert rows["id"] == [1, 2, 3]
        assert rows["category"] == ["factuality_csv", "reasoning_csv", "safety_csv"]

    def test_parquet_query(
        self, dch_client: DataConnectClient, s3_flight_connection: str, s3_parquet_query: str | None
    ) -> None:
        if not s3_parquet_query:
            pytest.skip("DCH_S3_PARQUET_QUERY not set")
        table = dch_client.read(s3_parquet_query, s3_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert rows["id"] == [11, 12, 13]
        assert rows["category"] == ["factuality_parquet", "reasoning_parquet", "safety_parquet"]

    def test_jsonl_query(
        self, dch_client: DataConnectClient, s3_flight_connection: str, s3_jsonl_query: str | None
    ) -> None:
        if not s3_jsonl_query:
            pytest.skip("DCH_S3_JSONL_QUERY not set")
        table = dch_client.read(s3_jsonl_query, s3_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert rows["id"] == [21, 22, 23]
        assert rows["category"] == ["factuality_jsonl", "reasoning_jsonl", "safety_jsonl"]
