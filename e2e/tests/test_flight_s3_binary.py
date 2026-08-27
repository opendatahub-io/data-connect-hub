"""Flight S3 binary download tests: download raw files from S3 via the binary protocol.

Uses the custom ``dataconnethub.opendatahub.io/download`` Flight command to
stream raw file bytes through ``get_flight_info`` / ``do_get``.

Requires AWS credentials in the env file and a binary test file on S3.
Skips automatically if S3 is not configured.
"""

from __future__ import annotations

import pyarrow as pa
import pyarrow.flight as flight
import pytest
from google.protobuf.any_pb2 import Any as PbAny

DOWNLOAD_TYPE_URL = "dataconnethub.opendatahub.io/download"


def _binary_download(
    client: flight.FlightClient,
    opts: flight.FlightCallOptions,
    path: str,
) -> bytes:
    cmd = PbAny()
    cmd.type_url = DOWNLOAD_TYPE_URL
    cmd.value = path.encode("utf-8")

    descriptor = flight.FlightDescriptor.for_command(cmd.SerializeToString())
    info = client.get_flight_info(descriptor, opts)
    assert len(info.endpoints) > 0

    reader = client.do_get(info.endpoints[0].ticket, opts)
    result = bytearray()
    for chunk in reader:
        col = chunk.data.column("data")
        for i in range(len(col)):
            result.extend(col[i].as_py())
    return bytes(result)


class TestFlightS3Binary:
    def test_binary_download(
        self,
        s3_flight_connection: str,
        s3_binary_path: str | None,
        flight_client_factory,
    ) -> None:
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set")

        client, opts = flight_client_factory(s3_flight_connection)
        try:
            data = _binary_download(client, opts, s3_binary_path)
            assert len(data) > 0
            assert data == b"binary-test-data-for-e2e\n"
        finally:
            client.close()

    def test_binary_download_schema(
        self,
        s3_flight_connection: str,
        s3_binary_path: str | None,
        flight_client_factory,
    ) -> None:
        """Verify the response schema is a single Binary column named 'data'."""
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set")

        client, opts = flight_client_factory(s3_flight_connection)
        try:
            cmd = PbAny()
            cmd.type_url = DOWNLOAD_TYPE_URL
            cmd.value = s3_binary_path.encode("utf-8")
            descriptor = flight.FlightDescriptor.for_command(cmd.SerializeToString())
            info = client.get_flight_info(descriptor, opts)

            schema = info.schema
            assert len(schema) == 1
            assert schema.field(0).name == "data"
            assert schema.field(0).type == pa.binary()
        finally:
            client.close()

    def test_binary_download_not_found(
        self,
        s3_flight_connection: str,
        s3_binary_path: str | None,
        flight_client_factory,
    ) -> None:
        """Attempting to download a non-existent file returns an error."""
        if not s3_binary_path:
            pytest.skip("DCH_S3_BINARY_PATH not set (need S3 configured)")

        client, opts = flight_client_factory(s3_flight_connection)
        try:
            cmd = PbAny()
            cmd.type_url = DOWNLOAD_TYPE_URL
            cmd.value = b"nonexistent/path/file.bin"
            descriptor = flight.FlightDescriptor.for_command(cmd.SerializeToString())

            with pytest.raises(flight.FlightError):
                client.get_flight_info(descriptor, opts)
        finally:
            client.close()
