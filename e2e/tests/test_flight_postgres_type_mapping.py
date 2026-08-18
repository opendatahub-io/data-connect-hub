"""Flight SQL type mapping: verify Arrow schema and values for PG types.

Uses literal SQL expressions (no table dependency) to test that
PostgreSQL types are correctly mapped to Arrow types through Flight.
"""

from __future__ import annotations

import datetime
import json

import pyarrow as pa

from data_connect_hub import DataConnectClient


class TestFlightPostgresTypeMapping:
    QUERY = """
        SELECT
            TIMESTAMP '2026-08-17 12:34:56.123456' AS ts,
            NULL::TIMESTAMP AS ts_null,
            TIMESTAMPTZ '2026-08-17 12:34:56.123456+00' AS tstz,
            NULL::TIMESTAMPTZ AS tstz_null,
            TIMESTAMPTZ '2026-08-17 20:34:56.123456+08' AS tstz_asia,
            DATE '2026-08-17' AS d,
            NULL::DATE AS d_null,
            TIME '12:34:56.123456' AS t,
            NULL::TIME AS t_null,
            TIMESTAMP '1970-01-01 00:00:00' AS ts_epoch,
            TIME '00:00:00' AS t_midnight,
            TIME '23:59:59.999999' AS t_end_of_day,
            '550e8400-e29b-41d4-a716-446655440000'::UUID AS u,
            '{"k":"v"}'::JSONB AS j,
            '12345.6789'::NUMERIC AS n
    """

    def _read(self, dch_client: DataConnectClient, pg_flight_connection: str) -> pa.Table:
        return dch_client.read(self.QUERY, pg_flight_connection)

    def test_timestamp_schema(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        table = self._read(dch_client, pg_flight_connection)

        ts_type = table.schema.field("ts").type
        assert pa.types.is_timestamp(ts_type) and ts_type.unit == "us"
        assert ts_type.tz is None

        ts_null_type = table.schema.field("ts_null").type
        assert pa.types.is_timestamp(ts_null_type) and ts_null_type.unit == "us"
        assert ts_null_type.tz is None

        ts_epoch_type = table.schema.field("ts_epoch").type
        assert pa.types.is_timestamp(ts_epoch_type) and ts_epoch_type.unit == "us"

    def test_timestamptz_schema(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        table = self._read(dch_client, pg_flight_connection)

        for col in ("tstz", "tstz_null", "tstz_asia"):
            col_type = table.schema.field(col).type
            assert pa.types.is_timestamp(col_type), f"{col}: expected timestamp, got {col_type}"
            assert col_type.unit == "us", f"{col}: expected us, got {col_type.unit}"
            assert col_type.tz == "UTC", f"{col}: expected UTC, got {col_type.tz}"

    def test_date_schema(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        table = self._read(dch_client, pg_flight_connection)
        for col in ("d", "d_null"):
            assert pa.types.is_date32(table.schema.field(col).type)

    def test_time_schema(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        table = self._read(dch_client, pg_flight_connection)
        for col in ("t", "t_null", "t_midnight", "t_end_of_day"):
            col_type = table.schema.field(col).type
            assert pa.types.is_time64(col_type), f"{col}: expected time64, got {col_type}"
            assert col_type.unit == "us", f"{col}: expected us, got {col_type.unit}"

    def test_string_types_schema(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        table = self._read(dch_client, pg_flight_connection)
        for col in ("u", "j", "n"):
            assert pa.types.is_string(table.schema.field(col).type), f"{col}: expected Utf8"

    def test_timestamp_values(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        row = self._read(dch_client, pg_flight_connection).to_pylist()[0]

        assert row["ts"] == datetime.datetime(2026, 8, 17, 12, 34, 56, 123456)
        assert row["ts_null"] is None
        assert row["ts_epoch"] == datetime.datetime(1970, 1, 1, 0, 0, 0)

    def test_timestamptz_values(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        row = self._read(dch_client, pg_flight_connection).to_pylist()[0]
        expected_utc = datetime.datetime(2026, 8, 17, 12, 34, 56, 123456, tzinfo=datetime.timezone.utc)

        assert row["tstz"] == expected_utc
        assert row["tstz_null"] is None
        assert row["tstz_asia"] == expected_utc
        assert row["tstz_asia"] == row["tstz"]

    def test_date_values(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        row = self._read(dch_client, pg_flight_connection).to_pylist()[0]

        assert row["d"] == datetime.date(2026, 8, 17)
        assert row["d_null"] is None

    def test_time_values(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        row = self._read(dch_client, pg_flight_connection).to_pylist()[0]

        assert row["t"] == datetime.time(12, 34, 56, 123456)
        assert row["t_null"] is None
        assert row["t_midnight"] == datetime.time(0, 0, 0)
        assert row["t_end_of_day"] == datetime.time(23, 59, 59, 999999)

    def test_string_type_values(self, dch_client: DataConnectClient, pg_flight_connection: str) -> None:
        row = self._read(dch_client, pg_flight_connection).to_pylist()[0]

        assert row["u"] == "550e8400-e29b-41d4-a716-446655440000"
        assert json.loads(row["j"]) == {"k": "v"}
        assert row["n"] == "12345.6789"
