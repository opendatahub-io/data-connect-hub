"""Flight Neo4j tests: query against a Neo4j graph database.

Requires Neo4j credentials in the env file and seeded test data
(City and Person nodes with relationships).
Skips automatically if Neo4j is not configured.
"""

from __future__ import annotations

import json

import pyarrow as pa

from data_connect_hub import DataConnectClient


class TestFlightNeo4j:
    def test_neo4j_return_all_cities(self, dch_client: DataConnectClient, neo4j_flight_connection: str) -> None:
        """Query all City nodes."""
        query = "MATCH (c:City) WHERE c.dch_e2e = true RETURN c.name AS name, c.country AS country, c.population AS population ORDER BY c.name"
        table = dch_client.read(query, neo4j_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        assert set(table.column_names) >= {"name", "country", "population"}
        rows = table.to_pydict()
        assert set(rows["name"]) == {"Tokyo", "London", "Paris", "New York", "Berlin"}

    def test_neo4j_filter_by_property(self, dch_client: DataConnectClient, neo4j_flight_connection: str) -> None:
        """Query with a property filter."""
        query = "MATCH (c:City) WHERE c.country = 'Japan' AND c.dch_e2e = true RETURN c.name AS name, c.population AS population"
        table = dch_client.read(query, neo4j_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 1
        rows = table.to_pydict()
        assert rows["name"][0] == "Tokyo"
        assert rows["population"][0] == 13960000

    def test_neo4j_return_persons(self, dch_client: DataConnectClient, neo4j_flight_connection: str) -> None:
        """Query Person nodes."""
        query = "MATCH (p:Person) WHERE p.dch_e2e = true RETURN p.name AS name, p.age AS age ORDER BY p.name"
        table = dch_client.read(query, neo4j_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        rows = table.to_pydict()
        assert set(rows["name"]) == {"Alice", "Bob", "Carol"}

    def test_neo4j_relationship_query(self, dch_client: DataConnectClient, neo4j_flight_connection: str) -> None:
        """Query across relationships."""
        query = "MATCH (p:Person)-[:LIVES_IN]->(c:City) WHERE p.dch_e2e = true RETURN p.name AS person, c.name AS city ORDER BY p.name"
        table = dch_client.read(query, neo4j_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        rows = table.to_pydict()
        pairs = set(zip(rows["person"], rows["city"]))
        assert ("Alice", "Tokyo") in pairs
        assert ("Bob", "London") in pairs
        assert ("Carol", "Paris") in pairs

    def test_neo4j_limit(self, dch_client: DataConnectClient, neo4j_flight_connection: str) -> None:
        """Verify LIMIT restricts result count."""
        query = "MATCH (c:City) WHERE c.dch_e2e = true RETURN c.name AS name LIMIT 2"
        table = dch_client.read(query, neo4j_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 2

    def test_neo4j_relationship_with_properties(self, dch_client: DataConnectClient, neo4j_flight_connection: str) -> None:
        """Query relationship properties."""
        query = "MATCH (a:City)-[f:FLIGHT_TO]->(b:City) WHERE a.dch_e2e = true RETURN a.name AS origin, b.name AS destination, f.distance_km AS distance_km ORDER BY f.distance_km"
        table = dch_client.read(query, neo4j_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 5
        rows = table.to_pydict()
        assert rows["distance_km"] == sorted(rows["distance_km"])
        assert rows["origin"][0] == "London"
        assert rows["destination"][0] == "Paris"
        assert rows["distance_km"][0] == 344

    def test_neo4j_return_node(self, dch_client: DataConnectClient, neo4j_flight_connection: str) -> None:
        """Return a full node — should be serialized as JSON string."""
        query = "MATCH (p:Person) WHERE p.name = 'Alice' AND p.dch_e2e = true RETURN p"
        table = dch_client.read(query, neo4j_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 1
        node_str = table.column("p")[0].as_py()
        node = json.loads(node_str)
        assert "Person" in node["labels"]
        assert node["properties"]["name"] == "Alice"
