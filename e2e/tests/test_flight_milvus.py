"""Flight Milvus tests: query and search against a Milvus collection.

Requires Milvus credentials in the env file and seeded test data.
Skips automatically if Milvus is not configured.
"""

from __future__ import annotations

import pyarrow as pa

from data_connect_hub import DataConnectClient

# Milvus REST request json format: https://github.com/milvus-io/web-content/tree/master/API_Reference/milvus-restful

class TestFlightMilvus:
    def test_milvus_query(self, dch_client: DataConnectClient, milvus_flight_connection: str) -> None:
        query = '{"collectionName":"dch_e2e_prompts","filter":"id > 0","outputFields":["id","category","prompt"],"limit":100}'
        table = dch_client.read(query, milvus_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert set(rows["category"]) == {"factuality", "reasoning", "safety"}

    def test_milvus_get(self, dch_client: DataConnectClient, milvus_flight_connection: str) -> None:
        query = '{"collectionName":"dch_e2e_prompts","id":[1,3],"outputFields":["id","category","prompt"]}'
        table = dch_client.read(query, milvus_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 2
        assert set(table.column_names) >= {"id", "category", "prompt"}
        rows = table.to_pydict()
        assert set(rows["id"]) == {1, 3}

    def test_milvus_search(self, dch_client: DataConnectClient, milvus_flight_connection: str) -> None:
        query = '{"collectionName":"dch_e2e_prompts","data":[[0.1,0.2,0.3,0.4]],"annsField":"embedding","limit":3,"outputFields":["id","category","prompt"]}'
        table = dch_client.read(query, milvus_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows <= 3
        assert table.num_rows > 0
        assert set(table.column_names) >= {"id", "category", "prompt"}
