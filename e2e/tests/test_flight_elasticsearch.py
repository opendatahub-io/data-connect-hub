"""Flight Elasticsearch tests: query against an Elasticsearch index.

Requires Elasticsearch credentials in the env file and seeded test data
(dch_e2e_cities index with 5 cities).
Skips automatically if Elasticsearch is not configured.
"""

from __future__ import annotations

import json

import pyarrow as pa

from data_connect_hub import DataConnectClient


class TestFlightElasticsearch:
    def test_es_match_all(self, dch_client: DataConnectClient, es_flight_connection: str) -> None:
        """Query all documents from the index."""
        query = json.dumps({
            "index": "dch_e2e_cities",
            "query": {"match_all": {}},
            "_source": ["name", "country", "population"],
        })
        table = dch_client.read(query, es_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert set(table.column_names) >= {"name", "country", "population"}
        rows = table.to_pydict()
        assert set(rows["name"]) == {"Tokyo", "London", "Paris", "New York", "Berlin"}

    def test_es_term_filter(self, dch_client: DataConnectClient, es_flight_connection: str) -> None:
        """Query with a term filter."""
        query = json.dumps({
            "index": "dch_e2e_cities",
            "query": {"term": {"country": "Japan"}},
            "_source": ["name", "country", "population"],
        })
        table = dch_client.read(query, es_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 1
        rows = table.to_pydict()
        assert rows["name"][0] == "Tokyo"
        assert rows["population"][0] == 13960000

    def test_es_range_query(self, dch_client: DataConnectClient, es_flight_connection: str) -> None:
        """Query with a range filter on population."""
        query = json.dumps({
            "index": "dch_e2e_cities",
            "query": {"range": {"population": {"gte": 8000000}}},
            "_source": ["name", "population"],
        })
        table = dch_client.read(query, es_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        rows = table.to_pydict()
        assert set(rows["name"]) == {"Tokyo", "London", "New York"}

    def test_es_size_limit(self, dch_client: DataConnectClient, es_flight_connection: str) -> None:
        """Verify size parameter limits total results."""
        query = json.dumps({
            "index": "dch_e2e_cities",
            "query": {"match_all": {}},
            "_source": ["name"],
            "size": 2,
        })
        table = dch_client.read(query, es_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 2

    def test_es_sorted_query(self, dch_client: DataConnectClient, es_flight_connection: str) -> None:
        """Query with explicit sort order."""
        query = json.dumps({
            "index": "dch_e2e_cities",
            "query": {"match_all": {}},
            "_source": ["name", "population"],
            "sort": [{"population": "desc"}],
            "size": 3,
        })
        table = dch_client.read(query, es_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        rows = table.to_pydict()
        populations = rows["population"]
        assert populations == sorted(populations, reverse=True)
        assert rows["name"][0] == "Tokyo"


class TestFlightElasticsearchApiKey:
    """Tests using API key authentication instead of basic auth."""

    def test_es_apikey_query(self, dch_client: DataConnectClient, es_apikey_flight_connection: str) -> None:
        """Verify API key auth works end-to-end with a real query."""
        query = json.dumps({
            "index": "dch_e2e_cities",
            "query": {"match_all": {}},
            "_source": ["name", "country"],
        })
        table = dch_client.read(query, es_apikey_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert set(table.column_names) >= {"name", "country"}
        rows = table.to_pydict()
        assert set(rows["name"]) == {"Tokyo", "London", "Paris", "New York", "Berlin"}
