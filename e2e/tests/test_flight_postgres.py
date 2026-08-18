"""Full data path: query seeded PG data via Flight SQL.

Requires run-e2e.sh to have seeded dch_e2e.cities and set DCH_PG_SECRET.
"""

from __future__ import annotations

import pyarrow as pa
import pytest

from data_connect_hub import DCHQueryError, DataConnectClient


class TestFlightPostgres:
    def test_select_all(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        table = dch_client.read("SELECT * FROM dch_e2e.cities ORDER BY id", pg_flight_connection)
        assert isinstance(table, pa.Table)
        assert table.num_rows == 3
        assert set(table.column_names) >= {"id", "name", "country", "population"}

    def test_read_pandas(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        df = dch_client.read_pandas("SELECT name FROM dch_e2e.cities ORDER BY id", pg_flight_connection)
        assert df["name"].tolist() == ["Tokyo", "London", "Paris"]

    def test_write_is_rejected(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        with pytest.raises(DCHQueryError, match="(?i)read-only"):
            dch_client.read("DELETE FROM dch_e2e.cities", pg_flight_connection)
