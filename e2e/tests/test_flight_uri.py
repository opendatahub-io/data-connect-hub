"""Flight URI tests: query data served over HTTP in multiple formats.

The URI connector detects format from the HTTP Content-Type header.
An explicit ``format`` field in the query JSON overrides Content-Type detection.

Requires a URI test server deployed in the cluster.
Skips automatically if URI is not configured.
"""

from __future__ import annotations

import json

import pyarrow as pa
from data_connect_hub import DataConnectClient

CITIES = {"Tokyo", "London", "Paris", "New York", "Berlin"}

POPULATION = {
    "Tokyo": 13960000,
    "London": 8982000,
    "Paris": 2161000,
    "New York": 8336000,
    "Berlin": 3645000,
}


class TestFlightUriJson:
    def test_json_get_all_cities(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """GET a top-level JSON array."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert set(table.column_names) >= {"name", "country", "population"}
        rows = table.to_pydict()
        assert set(rows["name"]) == CITIES

    def test_json_nested_data_path(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """GET with data_path to extract nested array."""
        query = json.dumps({"path": "/api/nested.json", "data_path": "data.items"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        rows = table.to_pydict()
        assert set(rows["name"]) == CITIES

    def test_json_schema_fields_sorted(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify schema fields are sorted alphabetically."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.column_names == sorted(table.column_names)

    def test_json_type_inference(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify JSON types are correctly mapped to Arrow types."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)

        schema = table.schema
        assert schema.field("name").type == pa.utf8()
        assert schema.field("country").type == pa.utf8()
        assert schema.field("population").type == pa.int64()
        assert schema.field("active").type == pa.bool_()

    def test_json_population_values(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify integer values are correctly parsed."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        rows = table.to_pydict()
        name_pop = dict(zip(rows["name"], rows["population"], strict=True))
        assert name_pop == POPULATION

    def test_json_boolean_values(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify boolean values are correctly parsed."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        rows = table.to_pydict()
        name_active = dict(zip(rows["name"], rows["active"], strict=True))
        assert name_active["Tokyo"] is True
        assert name_active["Berlin"] is False


class TestFlightUriCsv:
    def test_csv_basic(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """GET a CSV file — format detected from Content-Type text/csv."""
        query = json.dumps({"path": "/api/cities.csv"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert "name" in table.column_names
        assert "population" in table.column_names
        rows = table.to_pydict()
        assert set(rows["name"]) == CITIES

    def test_csv_population_values(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify CSV integer values are correctly parsed."""
        query = json.dumps({"path": "/api/cities.csv"})
        table = dch_client.read(query, uri_flight_connection)
        rows = table.to_pydict()
        name_pop = dict(zip(rows["name"], rows["population"], strict=True))
        assert name_pop["Tokyo"] == 13960000
        assert name_pop["Berlin"] == 3645000


class TestFlightUriJsonl:
    def test_jsonl_basic(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """GET a JSONL file — format detected from Content-Type application/x-ndjson."""
        query = json.dumps({"path": "/api/cities.jsonl"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert "name" in table.column_names
        assert "population" in table.column_names
        rows = table.to_pydict()
        assert set(rows["name"]) == CITIES

    def test_jsonl_type_mapping(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify JSONL types are correctly mapped to Arrow types."""
        query = json.dumps({"path": "/api/cities.jsonl"})
        table = dch_client.read(query, uri_flight_connection)
        schema = table.schema
        assert schema.field("name").type == pa.utf8()
        assert schema.field("country").type == pa.utf8()
        assert schema.field("population").type == pa.int64()

    def test_jsonl_population_values(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify JSONL integer values are correctly parsed."""
        query = json.dumps({"path": "/api/cities.jsonl"})
        table = dch_client.read(query, uri_flight_connection)
        rows = table.to_pydict()
        name_pop = dict(zip(rows["name"], rows["population"], strict=True))
        assert name_pop["Tokyo"] == 13960000
        assert name_pop["London"] == 8982000


class TestFlightUriParquet:
    def test_parquet_basic(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """GET a Parquet file — format detected from Content-Type."""
        query = json.dumps({"path": "/api/cities.parquet"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert "name" in table.column_names
        assert "population" in table.column_names
        rows = table.to_pydict()
        assert set(rows["name"]) == CITIES

    def test_parquet_type_mapping(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify Parquet types are preserved through the connector."""
        query = json.dumps({"path": "/api/cities.parquet"})
        table = dch_client.read(query, uri_flight_connection)
        schema = table.schema
        assert schema.field("name").type == pa.utf8()
        assert schema.field("country").type == pa.utf8()
        assert schema.field("population").type == pa.int64()
        assert schema.field("active").type == pa.bool_()

    def test_parquet_population_values(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify Parquet integer values are correctly parsed."""
        query = json.dumps({"path": "/api/cities.parquet"})
        table = dch_client.read(query, uri_flight_connection)
        rows = table.to_pydict()
        name_pop = dict(zip(rows["name"], rows["population"], strict=True))
        assert name_pop["Tokyo"] == 13960000
        assert name_pop["Paris"] == 2161000


class TestFlightUriFormatOverride:
    def test_explicit_format_csv(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Explicit 'format' field in query overrides Content-Type detection."""
        query = json.dumps({"path": "/api/cities.csv", "format": "csv"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        rows = table.to_pydict()
        assert set(rows["name"]) == CITIES

    def test_explicit_format_jsonl(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Explicit 'format' field works for JSONL."""
        query = json.dumps({"path": "/api/cities.jsonl", "format": "jsonl"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        rows = table.to_pydict()
        assert set(rows["name"]) == CITIES
