"""Flight DoAction tests for the three custom flight actions:

- ``CheckDataConnection``  (check a known data connection by id / inline creds)
- ``CheckCredentials``     (check credentials without a stored connection)
- ``GetSupportedConnectors`` (invoked directly via DoAction; its response
  body is an Arrow IPC stream of connector names), plus ``list_actions`` (which
  only lists the action declarations, not running them).

The REST ``readiness`` / ``test-credentials`` endpoints exercise the same
flight actions indirectly; these tests drive the flight ``DoAction`` RPC
directly so the action dispatch is covered end to end.
"""

from __future__ import annotations

import os

import pyarrow as pa
import pyarrow.flight as flight
import pytest
from data_connect_hub import DataConnectClient

CHECK_DATA_CONNECTION = "CheckDataConnection"
CHECK_CREDENTIALS = "CheckCredentials"
GET_SUPPORTED_CONNECTORS = "GetSupportedConnectors"


def _flight_client_and_options(
    dch_client: DataConnectClient,
) -> tuple[flight.FlightClient, flight.FlightCallOptions]:
    fc = dch_client._require_flight()
    client = fc._flight_connect()
    options = fc._call_options()
    return client, options


def _connection_headers(dch_client: DataConnectClient, connection_id: str) -> flight.FlightCallOptions:
    fc = dch_client._require_flight()
    headers = [
        *fc._call_options().headers,
        (b"x-data-connection-id", connection_id.encode()),
    ]
    return flight.FlightCallOptions(headers=headers)


def _credentials_body(dct_id: str, secret_uri: str, ca_cert: str | None = None) -> bytes:
    # Only emit the CA key when a certificate is supplied, so a plain
    # "URI only" body still exercises the no-SSL / require path.
    keys = ["data_connection_type_id", "secret.URI"]
    values = [dct_id, secret_uri]
    if ca_cert:
        keys.append("secret.CA_CERT")
        values.append(ca_cert)
    batch = pa.RecordBatch.from_arrays(
        [pa.array(keys, type=pa.utf8()), pa.array(values, type=pa.utf8())],
        names=["key", "value"],
    )

    sink = pa.BufferOutputStream()
    writer = pa.ipc.new_stream(sink, batch.schema)
    writer.write_batch(batch)
    writer.close()
    return sink.getvalue().to_pybytes()


def _get_supported_connectors(
    client: flight.FlightClient, options: flight.FlightCallOptions
) -> list[str]:
    """Run the GetSupportedConnectors action and return the connector names.

    The action body is an Arrow IPC stream with ``name``/``description``
    columns, so we decode ``result.body`` and read the ``name`` column.
    """
    results = list(client.do_action(flight.Action(GET_SUPPORTED_CONNECTORS, b""), options))
    if not results:
        return []
    body = results[0].body.to_pybytes()
    table = pa.ipc.open_stream(body).read_all()
    return table.column("name").to_pylist()


class TestFlightActions:
    def test_list_actions_includes_check_data_connection(
        self, dch_client: DataConnectClient
    ) -> None:
        client, options = _flight_client_and_options(dch_client)
        try:
            actions = list(client.list_actions(options))
            action_types = [a.type for a in actions]
            assert CHECK_DATA_CONNECTION in action_types
            assert GET_SUPPORTED_CONNECTORS in action_types
            assert CHECK_CREDENTIALS in action_types
        finally:
            client.close()

    def test_get_supported_connectors_returns_expected_connectors(
        self, dch_client: DataConnectClient
    ) -> None:
        """Actually run the GetSupportedConnectors action handler.

        Unlike ``list_actions`` (which only declares actions), this triggers
        ``action_get_supported_connectors`` and verifies the decoded response
        body genuinely contains the connector provider we expect (postgres,
        registered by the ``pg_flight_connection`` fixture).
        """
        client, options = _flight_client_and_options(dch_client)
        try:
            names = _get_supported_connectors(client, options)
            assert "postgres" in names
        finally:
            client.close()

    def test_check_data_connection_existing(
        self, dch_client: DataConnectClient, pg_flight_connection: str
    ) -> None:
        client = _flight_connect(dch_client)
        options = _connection_headers(dch_client, pg_flight_connection)
        try:
            results = list(client.do_action(flight.Action(CHECK_DATA_CONNECTION, b""), options))
            assert len(results) == 1
        finally:
            client.close()

    def test_check_data_connection_nonexistent_fails(
        self, dch_client: DataConnectClient
    ) -> None:
        client = _flight_connect(dch_client)
        options = _connection_headers(
            dch_client, "00000000-0000-0000-0000-000000000000"
        )
        try:
            with pytest.raises((flight.FlightServerError, pa.ArrowKeyError)):
                list(client.do_action(flight.Action(CHECK_DATA_CONNECTION, b""), options))
        finally:
            client.close()

    # Removed: this action ignores `Action.body` (resolves by
    # `x-data-connection-id`), so it never validated credentials; that is
    # genuinely covered by `test_check_credentials_action` instead.

    def test_check_credentials_action(
        self,
        dch_client: DataConnectClient,
        pg_flight_connection: str,
        rest_client: DataConnectClient,
    ) -> None:
        """Directly drive the CheckCredentials flight custom action.

        Unlike the CheckDataConnection action, CheckCredentials takes a
        key/value IPC body instead of relying on a stored connection id, so
        this exercises the credential-parsing + connector resolution path.
        """
        pg_url = os.environ.get("DCH_TENANT_PG_URL")
        if not pg_url:
            pytest.skip("DCH_TENANT_PG_URL not set (raw PG URL needed)")

        # When SSL is enabled with verification (sslmode=verify-ca), load the
        # tenant CA certificate so the postgres connector can verify the server
        # certificate. DCH_TENANT_PG_CA_CERT holds the local file path emitted
        # by __dch_generate_e2e_env; it may be empty for require/prefer modes.
        ca_cert = ""
        ca_cert_path = os.environ.get("DCH_TENANT_PG_CA_CERT")
        if ca_cert_path and os.path.exists(ca_cert_path):
            with open(ca_cert_path, encoding="utf-8") as f:
                ca_cert = f.read()

        conn = rest_client.get_connection(pg_flight_connection)
        dct_id = conn.data_connection_type_id

        body = _credentials_body(dct_id, pg_url, ca_cert)

        client = _flight_connect(dch_client)
        # No x-data-connection-id: CheckCredentials resolves by connection
        # type id instead.
        options = _flight_client_and_options(dch_client)[1]
        try:
            results = list(client.do_action(flight.Action(CHECK_CREDENTIALS, body), options))
            assert len(results) == 1
        finally:
            client.close()

    def test_unknown_action_fails(
        self, dch_client: DataConnectClient
    ) -> None:
        client, options = _flight_client_and_options(dch_client)
        try:
            with pytest.raises((flight.FlightServerError, pa.ArrowInvalid)):
                list(client.do_action(flight.Action("NoSuchAction", b""), options))
        finally:
            client.close()


def _flight_connect(dch_client: DataConnectClient) -> flight.FlightClient:
    return dch_client._require_flight()._flight_connect()
