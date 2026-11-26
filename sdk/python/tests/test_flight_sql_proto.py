"""Tests for the hand-written Flight SQL protobuf encoder (``_flight_sql_proto``).

Two independent checks:

- a minimal wire-format decoder (``conftest.unpack_any``) reads the fields back,
- ``TestGoldenProtobufEncoding`` compares the bytes with the official protobuf
  encoder (``protobuf`` is a dev dependency; those tests skip without it).

The encoder has no dependencies, so these tests don't need pyarrow.
"""

from __future__ import annotations

from typing import Any

import pytest

from data_connect_hub._flight_sql_proto import (
    CMD_GET_SQL_INFO_TYPE_URL,
    CMD_GET_TABLES_TYPE_URL,
    CMD_STATEMENT_QUERY_TYPE_URL,
    build_command_get_sql_info,
    build_command_get_tables,
    build_command_statement_query,
    encode_varint,
    pack_any,
)

from .conftest import decode_proto_fields, unpack_any

_STATEMENT_TYPE_URL = "type.googleapis.com/arrow.flight.protocol.sql.CommandStatementQuery"
_SQL_INFO_TYPE_URL = "type.googleapis.com/arrow.flight.protocol.sql.CommandGetSqlInfo"
_GET_TABLES_TYPE_URL = "type.googleapis.com/arrow.flight.protocol.sql.CommandGetTables"


def test_type_urls() -> None:
    assert CMD_STATEMENT_QUERY_TYPE_URL == _STATEMENT_TYPE_URL
    assert CMD_GET_SQL_INFO_TYPE_URL == _SQL_INFO_TYPE_URL
    assert CMD_GET_TABLES_TYPE_URL == _GET_TABLES_TYPE_URL


class TestEncodeVarint:
    def test_single_byte(self) -> None:
        assert encode_varint(0) == b"\x00"
        assert encode_varint(1) == b"\x01"
        assert encode_varint(127) == b"\x7f"

    def test_multi_byte(self) -> None:
        assert encode_varint(128) == b"\x80\x01"
        assert encode_varint(300) == b"\xac\x02"
        assert encode_varint(2**32 - 1) == b"\xff\xff\xff\xff\x0f"

    def test_negative_rejected(self) -> None:
        with pytest.raises(ValueError, match="non-negative"):
            encode_varint(-1)


class TestBuildCommandGetTables:
    def test_no_args(self) -> None:
        type_url, fields = unpack_any(build_command_get_tables())
        assert type_url == _GET_TABLES_TYPE_URL
        assert fields == {}

    def test_table_name_filter(self) -> None:
        _, fields = unpack_any(build_command_get_tables(table_name_filter_pattern="cities"))
        assert fields == {3: b"cities"}

    def test_include_schema(self) -> None:
        _, fields = unpack_any(build_command_get_tables(include_schema=True))
        assert fields == {5: 1}

    def test_both_args(self) -> None:
        _, fields = unpack_any(build_command_get_tables(table_name_filter_pattern="test%", include_schema=True))
        assert fields == {3: b"test%", 5: 1}

    def test_unicode_filter(self) -> None:
        _, fields = unpack_any(build_command_get_tables(table_name_filter_pattern="tést"))
        assert fields[3] == "tést".encode()


class TestBuildCommandStatementQuery:
    def test_query(self) -> None:
        type_url, fields = unpack_any(build_command_statement_query("SELECT 1"))
        assert type_url == _STATEMENT_TYPE_URL
        assert fields == {1: b"SELECT 1"}

    def test_long_query_uses_multibyte_length(self) -> None:
        sql = "SELECT " + ", ".join(f"c{i}" for i in range(200))
        _, fields = unpack_any(build_command_statement_query(sql))
        assert fields == {1: sql.encode()}


class TestBuildCommandGetSqlInfo:
    def test_ids_are_packed(self) -> None:
        type_url, fields = unpack_any(build_command_get_sql_info([0, 1, 2]))
        assert type_url == _SQL_INFO_TYPE_URL
        assert fields == {1: b"\x00\x01\x02"}

    def test_empty_requests_all(self) -> None:
        assert unpack_any(build_command_get_sql_info([]))[1] == {}


@pytest.fixture(scope="module")
def messages() -> dict[str, Any]:
    """Official protobuf classes for the Flight SQL commands the SDK encodes."""
    pytest.importorskip("google.protobuf")
    from google.protobuf import descriptor_pb2, descriptor_pool, message_factory

    field = descriptor_pb2.FieldDescriptorProto
    file_proto = descriptor_pb2.FileDescriptorProto(
        name="dch_test_flight_sql.proto", package="arrow.flight.protocol.sql", syntax="proto3"
    )

    def add(name: str, fields: list[tuple[str, int, int, int, bool]]) -> None:
        message = file_proto.message_type.add(name=name)
        for field_name, number, field_type, label, optional in fields:
            f = message.field.add(name=field_name, number=number, type=field_type, label=label)
            if optional:
                f.oneof_index = len(message.oneof_decl)
                f.proto3_optional = True
                message.oneof_decl.add(name=f"_{field_name}")

    single, repeated = field.LABEL_OPTIONAL, field.LABEL_REPEATED
    add(
        "CommandStatementQuery",
        [("query", 1, field.TYPE_STRING, single, False), ("transaction_id", 2, field.TYPE_BYTES, single, True)],
    )
    add("CommandGetSqlInfo", [("info", 1, field.TYPE_UINT32, repeated, False)])
    add(
        "CommandGetTables",
        [
            ("catalog", 1, field.TYPE_STRING, single, True),
            ("db_schema_filter_pattern", 2, field.TYPE_STRING, single, True),
            ("table_name_filter_pattern", 3, field.TYPE_STRING, single, True),
            ("table_types", 4, field.TYPE_STRING, repeated, False),
            ("include_schema", 5, field.TYPE_BOOL, single, False),
        ],
    )
    pool = descriptor_pool.DescriptorPool()
    pool.Add(file_proto)
    return {
        name: message_factory.GetMessageClass(pool.FindMessageTypeByName(f"arrow.flight.protocol.sql.{name}"))
        for name in ("CommandStatementQuery", "CommandGetSqlInfo", "CommandGetTables")
    }


class TestGoldenProtobufEncoding:
    """Byte-for-byte comparison with the official protobuf encoder.

    The message definitions below mirror FlightSql.proto (Apache Arrow); the
    server decodes the same field numbers with prost (``arrow-flight`` crate).
    """

    @staticmethod
    def _official(message: Any) -> bytes:
        from google.protobuf import any_pb2

        wrapper = any_pb2.Any()
        wrapper.Pack(message)
        return bytes(wrapper.SerializeToString())

    @pytest.mark.parametrize("sql", ["SELECT 1", "", "SELECT 'tést' -- ünïcode", "SELECT " + "x, " * 100 + "1"])
    def test_statement_query(self, messages: dict[str, Any], sql: str) -> None:
        expected = self._official(messages["CommandStatementQuery"](query=sql))
        assert build_command_statement_query(sql) == expected

    @pytest.mark.parametrize("ids", [[0, 1, 2], [], [0, 300, 10_000, 2**32 - 1]])
    def test_get_sql_info(self, messages: dict[str, Any], ids: list[int]) -> None:
        expected = self._official(messages["CommandGetSqlInfo"](info=ids))
        assert build_command_get_sql_info(ids) == expected

    @pytest.mark.parametrize(
        ("pattern", "include_schema"),
        [(None, False), ("cities", False), (None, True), ("test%", True), ("", False), ("tést", True)],
    )
    def test_get_tables(self, messages: dict[str, Any], pattern: str | None, include_schema: bool) -> None:
        kwargs: dict[str, Any] = {"include_schema": include_schema}
        if pattern is not None:
            kwargs["table_name_filter_pattern"] = pattern
        expected = self._official(messages["CommandGetTables"](**kwargs))
        assert build_command_get_tables(table_name_filter_pattern=pattern, include_schema=include_schema) == expected

    def test_pack_any_round_trips(self, messages: dict[str, Any]) -> None:
        from google.protobuf import any_pb2

        wrapper = any_pb2.Any()
        wrapper.ParseFromString(pack_any(_STATEMENT_TYPE_URL, b"\x0a\x01x"))
        decoded = messages["CommandStatementQuery"]()
        assert wrapper.Unpack(decoded)
        assert decoded.query == "x"


class TestStrictTestDecoder:
    """The test decoder must reject malformed input rather than return plausible values."""

    def test_truncated_length_delimited_field(self) -> None:
        # Field 1 declares 9 bytes but only "SELECT 1" (8 bytes) follows.
        with pytest.raises(ValueError, match="declares 9 bytes, 8 remain"):
            decode_proto_fields(b"\x0a\x09SELECT 1")

    def test_truncated_varint(self) -> None:
        with pytest.raises(ValueError, match="truncated varint"):
            decode_proto_fields(b"\x28\x80")

    def test_unsupported_wire_type(self) -> None:
        # Field 1 with wire type 5 (fixed32).
        with pytest.raises(ValueError, match="unsupported wire type 5"):
            decode_proto_fields(b"\x0d\x00\x00\x00\x00")
