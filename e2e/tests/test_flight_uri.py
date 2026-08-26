"""Flight URI tests: query against an HTTP/REST JSON endpoint.

Requires a URI test server deployed in the cluster and credentials
in the env file. Skips automatically if URI is not configured.
"""

from __future__ import annotations

import json

import pyarrow as pa
from data_connect_hub import DataConnectClient


class TestFlightUri:
    def test_uri_get_all_cities(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """GET a top-level JSON array."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert set(table.column_names) >= {"name", "country", "population"}
        rows = table.to_pydict()
        assert set(rows["name"]) == {"Tokyo", "London", "Paris", "New York", "Berlin"}

    def test_uri_nested_data_path(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """GET with data_path to extract nested array."""
        query = json.dumps({"path": "/api/nested.json", "data_path": "data.items"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        rows = table.to_pydict()
        assert set(rows["name"]) == {"Tokyo", "London", "Paris", "New York", "Berlin"}

    def test_uri_schema_fields_sorted(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify schema fields are sorted alphabetically."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.column_names == sorted(table.column_names)

    def test_uri_type_inference(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify JSON types are correctly mapped to Arrow types."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        assert isinstance(table, pa.Table)

        schema = table.schema
        assert schema.field("name").type == pa.utf8()
        assert schema.field("country").type == pa.utf8()
        assert schema.field("population").type == pa.int64()
        assert schema.field("active").type == pa.bool_()

    def test_uri_population_values(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify integer values are correctly parsed."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        rows = table.to_pydict()
        name_pop = dict(zip(rows["name"], rows["population"], strict=True))
        assert name_pop["Tokyo"] == 13960000
        assert name_pop["London"] == 8982000
        assert name_pop["Berlin"] == 3645000

    def test_uri_boolean_values(self, dch_client: DataConnectClient, uri_flight_connection: str) -> None:
        """Verify boolean values are correctly parsed."""
        query = json.dumps({"path": "/api/cities.json"})
        table = dch_client.read(query, uri_flight_connection)
        rows = table.to_pydict()
        name_active = dict(zip(rows["name"], rows["active"], strict=True))
        assert name_active["Tokyo"] is True
        assert name_active["Berlin"] is False
