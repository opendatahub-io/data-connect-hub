"""Full data path: query seeded PG data via Flight SQL.

Requires setup.sh to have seeded dch_e2e.cities and set DCH_E2E_PG_SECRET.
"""

from __future__ import annotations

import pyarrow as pa

from data_connect_hub import DataConnectClient


class TestFlightPostgres:
    def test_select_all(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        table = dch_client.read("SELECT * FROM dch_e2e.cities ORDER BY id", pg_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "name", "country", "population"}

    def test_read_pandas(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        df = dch_client.read_pandas("SELECT name FROM dch_e2e.cities ORDER BY id", pg_flight_connection)
        assert df["name"].tolist() == ["Tokyo", "London", "Paris"]
