"""Permission tests for the DCH data plane resources defined in ODH-ADR-0001.

All resources live in the ``dataconnecthub.opendatahub.io`` API group and are
checked in the tenant namespace:

- ``data-connections``: connection CRUD, readiness check, credential test
- ``data-connection-types``: connection type CRUD
- ``data-store``: data reads, i.e. REST binary download and every Flight call

run-e2e.sh creates one persona ServiceAccount per role in ``DCH_TENANT_ID``:

- reader: bound to the predefined ``dch-read`` ClusterRole
- metadata-only: ``get`` on data-connections and data-connection-types
- data-store-only: ``get`` on data-store
- connections-only: ``get`` and ``create`` on data-connections
- types-only: ``get`` and ``create`` on data-connection-types

REST is authorized by kube-rbac-proxy before a request reaches the service, so
a 403 on a missing resource proves the proxy denied it, and a 404 proves the
proxy let it through.
"""

from __future__ import annotations

import contextlib
import json
import uuid
from collections.abc import Callable

import httpx
import pytest

from data_connect_hub import (
    ConnectionType,
    CredentialsRef,
    DataConnectClient,
    DataConnection,
    DCHAuthenticationError,
    DCHConnectionError,
    DCHForbiddenError,
    DCHHTTPError,
    DCHNotFoundError,
    DCHValidationError,
)

_FLIGHT_DENIED = r"(?i)unauthorized|permission.?denied"


@pytest.fixture(scope="module")
def make_client(gateway_endpoint: str, tenant_id: str, ca_cert: str | None, insecure: bool):
    """Build a tenant client for *token*; skip when the persona is not configured."""
    clients: list[DataConnectClient] = []

    def _make(token: str, env_var: str) -> DataConnectClient:
        if not token:
            pytest.skip(f"{env_var} not set")
        client = DataConnectClient(
            gateway_endpoint,
            token=token,
            tenant_id=tenant_id,
            ca_cert=ca_cert,
            insecure=insecure,
            max_retries=3,
            rest_timeout=15.0,
        )
        clients.append(client)
        return client

    yield _make
    for client in clients:
        client.close()


@pytest.fixture(scope="module")
def reader_client(make_client, persona_reader_auth_token: str) -> DataConnectClient:
    return make_client(persona_reader_auth_token, "DCH_PERSONA_READER_AUTH_TOKEN")


@pytest.fixture(scope="module")
def metadata_only_client(make_client, persona_metadata_only_auth_token: str) -> DataConnectClient:
    return make_client(persona_metadata_only_auth_token, "DCH_PERSONA_METADATA_ONLY_AUTH_TOKEN")


@pytest.fixture(scope="module")
def data_store_only_client(make_client, persona_data_store_only_auth_token: str) -> DataConnectClient:
    return make_client(persona_data_store_only_auth_token, "DCH_PERSONA_DATA_STORE_ONLY_AUTH_TOKEN")


@pytest.fixture(scope="module")
def connections_only_client(make_client, persona_connections_only_auth_token: str) -> DataConnectClient:
    return make_client(persona_connections_only_auth_token, "DCH_PERSONA_CONNECTIONS_ONLY_AUTH_TOKEN")


@pytest.fixture(scope="module")
def types_only_client(make_client, persona_types_only_auth_token: str) -> DataConnectClient:
    return make_client(persona_types_only_auth_token, "DCH_PERSONA_TYPES_ONLY_AUTH_TOKEN")


@pytest.fixture()
def connection_type(create_connection_type) -> ConnectionType:
    """A connection type created by the read-write e2e user."""
    return create_connection_type(provider="postgres")


@pytest.fixture()
def connection(create_connection, connection_type: ConnectionType) -> DataConnection:
    """A connection created by the read-write e2e user.

    The secret it references does not exist; no test here reads through it.
    """
    return create_connection(
        connection_type_id=connection_type.id,
        credentials_ref=CredentialsRef(secret="dch-e2e-rbac-unused"),  # notsecret
    )


def _missing_id() -> str:
    return str(uuid.uuid4())


def _download_binary(client: DataConnectClient, connection_id: str) -> None:
    # download_binary is lazy: the request is sent on first iteration.
    for _ in client.download_binary(connection_id, "dch-e2e-rbac.bin"):
        pass


def _unique_name() -> str:
    return f"e2e-rbac-{uuid.uuid4().hex[:8]}"


def _assert_reaches_service(call: Callable[[], object]) -> None:
    """Assert kube-rbac-proxy let *call* through to the REST service.

    The service itself may still reject the request (the fixtures point at a
    secret that does not exist), so an error counts as success only if its body
    is a REST service error. A 401 or 403, or any body without a REST ``code``
    (gateway 404 or 5xx, OAuth redirect), is a failure.
    """
    try:
        call()
    except (DCHAuthenticationError, DCHForbiddenError) as exc:
        pytest.fail(f"kube-rbac-proxy rejected the request: {exc}")
    except DCHHTTPError as exc:
        if _rest_error_code(exc.body) is None:
            pytest.fail(f"response did not come from the REST service: {exc}")


def _rest_error_code(body: str) -> str | None:
    """Return the ``code`` of a REST service error body, or None for any other body."""
    try:
        parsed = json.loads(body)
    except ValueError:
        return None
    if isinstance(parsed, dict) and isinstance(parsed.get("code"), str) and "message" in parsed:
        return parsed["code"]
    return None


# Reader-denied writes, as (client, connection type, connection) -> object.
_Write = Callable[[DataConnectClient, ConnectionType, DataConnection], object]

# Writes that target the fixture resources, so their state can be re-checked after the denial.
_READER_DENIED_MUTATIONS: dict[str, _Write] = {
    "update_connection": lambda c, _, conn: c.update_connection(conn.id, name="e2e-rbac-renamed"),
    "delete_connection": lambda c, _, conn: c.delete_connection(conn.id),
    "update_connection_type": lambda c, ct, _: c.update_connection_type(ct.id, description="e2e-rbac"),
    "delete_connection_type": lambda c, ct, _: c.delete_connection_type(ct.id),
}

# Writes with effects outside the metadata store (Flight calls, Secret writes).
# kube-rbac-proxy rejects them before they reach the REST service, so the 403
# is the whole assertion.
_READER_DENIED_ACTIONS: dict[str, _Write] = {
    "check_connection_readiness": lambda c, _, conn: c.check_connection_readiness(conn.id),
    "test_credentials": lambda c, ct, _: c.test_credentials(ct.id, {"URI": "postgresql://unused"}),
    "export_connection": lambda c, _, conn: c.export_connection(conn.id, "dch-e2e-rbac-export"),
}

# Actions kube-rbac-proxy authorizes as `create` on data-connections.
_CONNECTION_CREATE_ACTIONS: dict[str, _Write] = {
    "check_connection_readiness": _READER_DENIED_ACTIONS["check_connection_readiness"],
    "test_credentials": _READER_DENIED_ACTIONS["test_credentials"],
}


class TestReaderRole:
    """``dch-read``: read connection metadata and data, change nothing."""

    def test_can_list_and_get_connections(self, reader_client: DataConnectClient, connection: DataConnection) -> None:
        assert connection.id in [c.id for c in reader_client.list_connections()]
        assert reader_client.get_connection(connection.id).id == connection.id

    def test_can_list_and_get_connection_types(
        self, reader_client: DataConnectClient, connection_type: ConnectionType
    ) -> None:
        assert connection_type.id in [ct.id for ct in reader_client.list_connection_types()]
        assert reader_client.get_connection_type(connection_type.id).id == connection_type.id

    def test_create_connection_is_denied(
        self,
        reader_client: DataConnectClient,
        rest_client: DataConnectClient,
        connection_type: ConnectionType,
    ) -> None:
        name = _unique_name()
        created: DataConnection | None = None
        try:
            with pytest.raises(DCHForbiddenError):
                created = reader_client.create_connection(
                    name=name,
                    connection_type_id=connection_type.id,
                    data_format="tabular",
                    credentials_ref=CredentialsRef(secret="dch-e2e-rbac-unused"),  # notsecret
                )
        finally:
            # Only reached if authorization is broken; keep the tenant clean.
            if created is not None:
                with contextlib.suppress(Exception):
                    rest_client.delete_connection(created.id)

        assert name not in [c.name for c in rest_client.list_connections()]

    def test_create_connection_type_is_denied(
        self,
        reader_client: DataConnectClient,
        rest_client: DataConnectClient,
    ) -> None:
        name = _unique_name()
        created: ConnectionType | None = None
        try:
            with pytest.raises(DCHForbiddenError):
                created = reader_client.create_connection_type(name=name, provider="postgres")
        finally:
            # Only reached if authorization is broken; keep the tenant clean.
            if created is not None:
                with contextlib.suppress(Exception):
                    rest_client.delete_connection_type(created.id)

        assert name not in [ct.name for ct in rest_client.list_connection_types()]

    @pytest.mark.parametrize("operation", list(_READER_DENIED_MUTATIONS))
    def test_mutation_is_denied(
        self,
        operation: str,
        reader_client: DataConnectClient,
        rest_client: DataConnectClient,
        connection_type: ConnectionType,
        connection: DataConnection,
    ) -> None:
        with pytest.raises(DCHForbiddenError):
            _READER_DENIED_MUTATIONS[operation](reader_client, connection_type, connection)

        # Both fixture resources still exist, unchanged.
        assert rest_client.get_connection(connection.id).name == connection.name
        assert rest_client.get_connection_type(connection_type.id).description == connection_type.description

    @pytest.mark.parametrize("operation", list(_READER_DENIED_ACTIONS))
    def test_action_is_denied(
        self,
        operation: str,
        reader_client: DataConnectClient,
        connection_type: ConnectionType,
        connection: DataConnection,
    ) -> None:
        with pytest.raises(DCHForbiddenError):
            _READER_DENIED_ACTIONS[operation](reader_client, connection_type, connection)

    def test_binary_download_is_authorized(self, reader_client: DataConnectClient) -> None:
        with pytest.raises(DCHNotFoundError):
            _download_binary(reader_client, _missing_id())

    def test_flight_is_authorized(self, reader_client: DataConnectClient) -> None:
        assert isinstance(reader_client.server_info(), dict)


class TestMetadataOnlyRole:
    """Connection metadata without ``data-store`` must not grant data reads."""

    def test_can_list_connections(self, metadata_only_client: DataConnectClient, connection: DataConnection) -> None:
        assert connection.id in [c.id for c in metadata_only_client.list_connections()]

    def test_flight_is_denied(self, metadata_only_client: DataConnectClient) -> None:
        with pytest.raises(DCHConnectionError, match=_FLIGHT_DENIED):
            metadata_only_client.server_info()

    def test_binary_download_is_denied(self, metadata_only_client: DataConnectClient) -> None:
        with pytest.raises(DCHForbiddenError):
            _download_binary(metadata_only_client, _missing_id())


class TestDataStoreOnlyRole:
    """``data-store`` grants data reads but not connection metadata."""

    def test_flight_is_authorized(self, data_store_only_client: DataConnectClient) -> None:
        assert isinstance(data_store_only_client.server_info(), dict)

    def test_binary_download_is_authorized(self, data_store_only_client: DataConnectClient) -> None:
        with pytest.raises(DCHNotFoundError):
            _download_binary(data_store_only_client, _missing_id())

    def test_list_connections_is_denied(self, data_store_only_client: DataConnectClient) -> None:
        with pytest.raises(DCHForbiddenError):
            data_store_only_client.list_connections()

    def test_list_connection_types_is_denied(self, data_store_only_client: DataConnectClient) -> None:
        with pytest.raises(DCHForbiddenError):
            data_store_only_client.list_connection_types()


class TestConnectionsOnlyRole:
    """``data-connections`` alone covers connections, readiness, and credential tests, not connection types."""

    def test_can_list_and_get_connections(
        self, connections_only_client: DataConnectClient, connection: DataConnection
    ) -> None:
        assert connection.id in [c.id for c in connections_only_client.list_connections()]
        assert connections_only_client.get_connection(connection.id).id == connection.id

    def test_connection_type_reads_are_denied(
        self, connections_only_client: DataConnectClient, connection_type: ConnectionType
    ) -> None:
        with pytest.raises(DCHForbiddenError):
            connections_only_client.list_connection_types()
        with pytest.raises(DCHForbiddenError):
            connections_only_client.get_connection_type(connection_type.id)

    def test_create_connection_type_is_denied(
        self, connections_only_client: DataConnectClient, rest_client: DataConnectClient
    ) -> None:
        name = _unique_name()
        created: ConnectionType | None = None
        try:
            with pytest.raises(DCHForbiddenError):
                created = connections_only_client.create_connection_type(name=name, provider="postgres")
        finally:
            # Only reached if authorization is broken; keep the tenant clean.
            if created is not None:
                with contextlib.suppress(Exception):
                    rest_client.delete_connection_type(created.id)

        assert name not in [ct.name for ct in rest_client.list_connection_types()]

    def test_create_connection_is_authorized(
        self,
        connections_only_client: DataConnectClient,
        rest_client: DataConnectClient,
        connection_type: ConnectionType,
    ) -> None:
        created = connections_only_client.create_connection(
            name=_unique_name(),
            connection_type_id=connection_type.id,
            data_format="tabular",
            credentials_ref=CredentialsRef(secret="dch-e2e-rbac-unused"),  # notsecret
        )
        try:
            assert rest_client.get_connection(created.id).id == created.id
        finally:
            rest_client.delete_connection(created.id)

    def test_readiness_is_authorized(
        self, connections_only_client: DataConnectClient, connection: DataConnection
    ) -> None:
        # The referenced secret does not exist, so the REST service itself
        # answers invalid_secret: proof that the proxy let the request through.
        with pytest.raises(DCHValidationError) as exc_info:
            connections_only_client.check_connection_readiness(connection.id)
        assert "invalid_secret" in exc_info.value.body

    def test_test_credentials_is_authorized(
        self, connections_only_client: DataConnectClient, connection_type: ConnectionType
    ) -> None:
        _assert_reaches_service(
            lambda: connections_only_client.test_credentials(connection_type.id, {"URI": "postgresql://unused"})
        )


class TestTypesOnlyRole:
    """``data-connection-types`` alone covers connection types, not connections or connection actions."""

    def test_can_list_and_get_connection_types(
        self, types_only_client: DataConnectClient, connection_type: ConnectionType
    ) -> None:
        assert connection_type.id in [ct.id for ct in types_only_client.list_connection_types()]
        assert types_only_client.get_connection_type(connection_type.id).id == connection_type.id

    def test_connection_reads_are_denied(
        self, types_only_client: DataConnectClient, connection: DataConnection
    ) -> None:
        with pytest.raises(DCHForbiddenError):
            types_only_client.list_connections()
        with pytest.raises(DCHForbiddenError):
            types_only_client.get_connection(connection.id)

    def test_create_connection_is_denied(
        self,
        types_only_client: DataConnectClient,
        rest_client: DataConnectClient,
        connection_type: ConnectionType,
    ) -> None:
        name = _unique_name()
        created: DataConnection | None = None
        try:
            with pytest.raises(DCHForbiddenError):
                created = types_only_client.create_connection(
                    name=name,
                    connection_type_id=connection_type.id,
                    data_format="tabular",
                    credentials_ref=CredentialsRef(secret="dch-e2e-rbac-unused"),  # notsecret
                )
        finally:
            # Only reached if authorization is broken; keep the tenant clean.
            if created is not None:
                with contextlib.suppress(Exception):
                    rest_client.delete_connection(created.id)

        assert name not in [c.name for c in rest_client.list_connections()]

    def test_create_connection_type_is_authorized(
        self, types_only_client: DataConnectClient, rest_client: DataConnectClient
    ) -> None:
        created = types_only_client.create_connection_type(name=_unique_name(), provider="postgres")
        try:
            assert rest_client.get_connection_type(created.id).id == created.id
        finally:
            rest_client.delete_connection_type(created.id)

    @pytest.mark.parametrize("operation", list(_CONNECTION_CREATE_ACTIONS))
    def test_connection_action_is_denied(
        self,
        operation: str,
        types_only_client: DataConnectClient,
        connection_type: ConnectionType,
        connection: DataConnection,
    ) -> None:
        # Readiness and credential tests require `create` on data-connections;
        # `create` on data-connection-types must not stand in for it.
        with pytest.raises(DCHForbiddenError):
            _CONNECTION_CREATE_ACTIONS[operation](types_only_client, connection_type, connection)


_FLIGHT_REGISTRATION_URL = "grpc+tls://dch-e2e-rbac.invalid:443"

# Internal flight service registration endpoints, called only by the controller.
_FLIGHT_REGISTRATION_REQUESTS: dict[str, tuple[str, str, dict[str, str] | None]] = {
    "register": (
        "POST",
        "/api/v1alpha1/data/flights",
        {
            "name": "dch-e2e-rbac-flight",
            "namespace": "dch-e2e-rbac",
            "external_url": _FLIGHT_REGISTRATION_URL,
            "internal_url": _FLIGHT_REGISTRATION_URL,
        },
    ),
    "update": ("PATCH", "/api/v1alpha1/data/flights/{id}", {"internal_url": _FLIGHT_REGISTRATION_URL}),
    "delete": ("DELETE", "/api/v1alpha1/data/flights/{id}", None),
}


class TestFlightRegistrationIsInternal:
    """Users cannot register, update, or delete flight services.

    A registered flight service receives the credentials of every connection
    it serves, so these endpoints are reserved for the controller: the
    HTTPRoute does not route them, and kube-rbac-proxy requires
    ``flight-services``, which no predefined role grants. The e2e user holds
    ``dch-read-write``. Whichever layer answers (the gateway with 404 for the
    unrouted path, kube-rbac-proxy with 403), the request must not reach the
    REST service, whose error bodies carry a ``code``. Any other status, such
    as an OAuth redirect or a gateway 5xx, means authorization was never tested.
    """

    @pytest.mark.parametrize("operation", list(_FLIGHT_REGISTRATION_REQUESTS))
    def test_request_does_not_reach_rest_service(
        self, operation: str, http_client: httpx.Client, auth_token: str, tenant_id: str
    ) -> None:
        method, path, body = _FLIGHT_REGISTRATION_REQUESTS[operation]
        resp = http_client.request(
            method,
            path.format(id=_missing_id()),
            headers={"Authorization": f"Bearer {auth_token}", "X-Tenant-Id": tenant_id},
            json=body,
        )
        assert resp.status_code in (403, 404), (
            f"{method} {path}: expected a gateway 404 or kube-rbac-proxy 403, got {resp.status_code} {resp.text}"
        )
        code = _rest_error_code(resp.text)
        assert code is None, f"{method} {path} reached the REST service ({code}): {resp.status_code} {resp.text}"
