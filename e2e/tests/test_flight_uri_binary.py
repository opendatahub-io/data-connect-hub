"""Flight URI binary download tests: download raw files from HTTP via the binary protocol.

Uses the custom ``dataconnethub.opendatahub.io/download`` Flight command to
stream raw file bytes through ``get_flight_info`` / ``do_get``.

Requires a URI test server deployed in the cluster.
Skips automatically if URI is not configured.
"""

from __future__ import annotations

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


class TestFlightUriBinary:
    def test_binary_download(
        self,
        uri_flight_connection: str,
        flight_client_factory,
    ) -> None:
        client, opts = flight_client_factory(uri_flight_connection)
        try:
            data = _binary_download(client, opts, "api/binary.dat")
            assert len(data) > 0
            assert data == b"binary-test-data-for-e2e\n"
        finally:
            client.close()

    def test_binary_download_not_found(
        self,
        uri_flight_connection: str,
        flight_client_factory,
    ) -> None:
        """Attempting to download a non-existent path returns an error."""
        client, opts = flight_client_factory(uri_flight_connection)
        try:
            cmd = PbAny()
            cmd.type_url = DOWNLOAD_TYPE_URL
            cmd.value = b"nonexistent/path.bin"
            descriptor = flight.FlightDescriptor.for_command(cmd.SerializeToString())

            with pytest.raises(flight.FlightError):
                client.get_flight_info(descriptor, opts)
        finally:
            client.close()
