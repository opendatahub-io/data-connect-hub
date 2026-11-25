"""Protobuf encoding for the Flight SQL commands the SDK sends.

The SDK sends three Flight SQL commands, each wrapped in
``google.protobuf.Any``. They are encoded by hand here so the ``flight`` extra
doesn't depend on ``protobuf`` or on generated ``FlightSql_pb2`` code.

Field numbers come from ``FlightSql.proto`` (Apache Arrow); the server decodes
them with the ``arrow-flight`` crate. The encoding follows the protobuf wire
format and matches what the official library emits:

- proto3 scalar fields are omitted when empty,
- ``optional`` fields are written whenever they are set,
- repeated scalars are packed.

``tests/test_flight_sql_proto.py`` compares the output byte for byte with the
official protobuf encoder. If the SDK needs many more or nested messages
(prepared statements, parameter binding, transactions), switch to generated
code instead of extending this module.

This module has no dependencies, so it can be tested without pyarrow.
"""

from __future__ import annotations

_TYPE_URL_PREFIX = "type.googleapis.com/arrow.flight.protocol.sql."
CMD_STATEMENT_QUERY_TYPE_URL = _TYPE_URL_PREFIX + "CommandStatementQuery"
CMD_GET_SQL_INFO_TYPE_URL = _TYPE_URL_PREFIX + "CommandGetSqlInfo"
CMD_GET_TABLES_TYPE_URL = _TYPE_URL_PREFIX + "CommandGetTables"

_WIRE_VARINT = 0
_WIRE_LEN = 2

# google.protobuf.Any
_ANY_FIELD_TYPE_URL = 1
_ANY_FIELD_VALUE = 2
# CommandStatementQuery
_CSQ_FIELD_QUERY = 1
# CommandGetSqlInfo
_CGSI_FIELD_INFO = 1
# CommandGetTables
_CGT_FIELD_TABLE_NAME_FILTER = 3
_CGT_FIELD_INCLUDE_SCHEMA = 5


def encode_varint(value: int) -> bytes:
    """Encode a non-negative integer as a protobuf base-128 varint."""
    if value < 0:
        raise ValueError("varint value must be non-negative")
    result = bytearray()
    while value > 0x7F:
        result.append((value & 0x7F) | 0x80)
        value >>= 7
    result.append(value & 0x7F)
    return bytes(result)


def _encode_tag(field_number: int, wire_type: int) -> bytes:
    return encode_varint((field_number << 3) | wire_type)


def _encode_len_field(field_number: int, data: bytes) -> bytes:
    return _encode_tag(field_number, _WIRE_LEN) + encode_varint(len(data)) + data


def pack_any(type_url: str, value: bytes) -> bytes:
    """Serialize a ``google.protobuf.Any`` holding *value* (an encoded message of type *type_url*)."""
    out = _encode_len_field(_ANY_FIELD_TYPE_URL, type_url.encode("utf-8"))
    if value:
        out += _encode_len_field(_ANY_FIELD_VALUE, value)
    return out


def build_command_statement_query(query: str) -> bytes:
    """Serialize a Flight SQL ``CommandStatementQuery`` wrapped in ``google.protobuf.Any``."""
    msg = _encode_len_field(_CSQ_FIELD_QUERY, query.encode("utf-8")) if query else b""
    return pack_any(CMD_STATEMENT_QUERY_TYPE_URL, msg)


def build_command_get_sql_info(info_ids: list[int]) -> bytes:
    """Serialize a Flight SQL ``CommandGetSqlInfo`` wrapped in ``google.protobuf.Any``."""
    msg = b""
    if info_ids:
        msg = _encode_len_field(_CGSI_FIELD_INFO, b"".join(encode_varint(i) for i in info_ids))
    return pack_any(CMD_GET_SQL_INFO_TYPE_URL, msg)


def build_command_get_tables(
    *,
    table_name_filter_pattern: str | None = None,
    include_schema: bool = False,
) -> bytes:
    """Serialize a Flight SQL ``CommandGetTables`` wrapped in ``google.protobuf.Any``."""
    msg = b""
    if table_name_filter_pattern is not None:
        msg += _encode_len_field(_CGT_FIELD_TABLE_NAME_FILTER, table_name_filter_pattern.encode("utf-8"))
    if include_schema:
        msg += _encode_tag(_CGT_FIELD_INCLUDE_SCHEMA, _WIRE_VARINT) + encode_varint(1)
    return pack_any(CMD_GET_TABLES_TYPE_URL, msg)
