"""Tenancy tests: ``X-Tenant-Id`` scopes every REST and Flight request.

Three namespaces take part, all prepared by run-e2e.sh:

- tenant (``DCH_TENANT_ID``): the e2e user's home tenant.
- neighbor tenant (``DCH_NEIGHBOR_TENANT_ID``): a second tenant on the same
  DCH service where the e2e user holds the same roles. Isolation tests create
  a resource in the tenant and reach for it with ``X-Tenant-Id`` set to the
  neighbor. Authorization passes in both tenants, so a leak could only come
  from missing tenant scoping in the metadata store or in secret lookups.
- no-access namespace (``DCH_NO_ACCESS_NAMESPACE``): the e2e user has no
  RoleBinding there, so the SubjectAccessReview must deny every request.
"""

from __future__ import annotations

import json
import subprocess
import uuid

import httpx
import pyarrow as pa
import pyarrow.flight as flight
import pytest

from data_connect_hub import (
    ConnectionType,
    CredentialField,
    CredentialsRef,
    DataConnectClient,
    DataConnection,
    DCHConflictError,
    DCHConnectionError,
    DCHForbiddenError,
    DCHNotFoundError,
    DCHValidationError,
    InlineCredentials,
)

CHECK_DATA_CONNECTION = "CheckDataConnection"
GLOBAL_TYPE_NAME = "Postgres"
UNUSED_SECRET = "dch-e2e-tenancy-unused"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _uri_field() -> list[CredentialField]:
    return [CredentialField(name="URI", label="URI", required=True, type="string")]


def _flight_headers(client: DataConnectClient) -> list[tuple[bytes, bytes]]:
    """Return the Flight request headers the SDK sends for *client*.

    Built from the SDK header dict because ``FlightCallOptions.headers`` is not
    readable on older PyArrow releases.
    """
    return [(k.lower().encode(), v.encode()) for k, v in client._require_flight()._headers().items()]


def _check_data_connection(client: DataConnectClient, connection_id: str) -> None:
    """Run the Flight ``CheckDataConnection`` action in *client*'s tenant."""
    raw = client._require_flight()._flight_connect()
    options = flight.FlightCallOptions(
        headers=[*_flight_headers(client), (b"x-data-connection-id", connection_id.encode())]
    )
    try:
        list(raw.do_action(flight.Action(CHECK_DATA_CONNECTION, b""), options))
    finally:
        raw.close()


def _secret_exists(namespace: str, name: str) -> bool:
    result = subprocess.run(
        ["kubectl", "get", "secret", name, "--namespace", namespace, "--ignore-not-found", "-o", "name"],
        check=True,
        capture_output=True,
        text=True,
    )
    return bool(result.stdout.strip())


def _global_type(client: DataConnectClient) -> ConnectionType:
    # Global types come back without a tenant_id, which tells them apart from
    # a tenant-scoped type with the same name.
    ct = next(
        (ct for ct in client.list_connection_types() if ct.name == GLOBAL_TYPE_NAME and not ct.tenant_id),
        None,
    )
    assert ct is not None, f"out-of-box connection type {GLOBAL_TYPE_NAME!r} not found"
    return ct


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture()
def tenant_type(create_connection_type) -> ConnectionType:
    """A tenant-scoped connection type owned by the tenant."""
    return create_connection_type(provider="postgres", credentials_fields=_uri_field())


@pytest.fixture()
def tenant_connection(create_connection, tenant_type: ConnectionType) -> DataConnection:
    """A connection owned by the tenant. Its secret need not exist."""
    return create_connection(
        connection_type_id=tenant_type.id,
        credentials_ref=CredentialsRef(secret=UNUSED_SECRET),
        properties={"owner": "tenant"},
    )


# ---------------------------------------------------------------------------
# X-Tenant-Id is required
# ---------------------------------------------------------------------------


class TestTenantHeaderRequired:
    """Requests without a tenant are rejected by the server.

    These send raw requests because the SDK refuses to send a request without
    ``tenant_id`` before anything reaches the server.
    """

    @pytest.mark.parametrize("headers", [{}, {"X-Tenant-Id": ""}], ids=["missing", "empty"])
    def test_rest_rejects_request_without_tenant(
        self, http_client: httpx.Client, auth_token: str, headers: dict[str, str]
    ) -> None:
        resp = http_client.get(
            "/api/v1alpha1/data/connections",
            headers={"Authorization": f"Bearer {auth_token}", **headers},
        )
        # kube-rbac-proxy (403, SubjectAccessReview without a namespace) or the
        # REST header check (400) may reject first; either way no tenant data
        # is returned.
        assert resp.status_code in (400, 403), resp.text

    def test_flight_rejects_request_without_tenant(self, dch_client: DataConnectClient) -> None:
        raw = dch_client._require_flight()._flight_connect()
        headers = [(k, v) for k, v in _flight_headers(dch_client) if k != b"x-tenant-id"]
        try:
            with pytest.raises(flight.FlightUnauthorizedError, match="x-tenant-id header is required"):
                list(raw.list_actions(flight.FlightCallOptions(headers=headers)))
        finally:
            raw.close()


# ---------------------------------------------------------------------------
# Namespace without RBAC
# ---------------------------------------------------------------------------


class TestNoAccessNamespace:
    """A tenant the caller holds no RoleBinding in is denied by SubjectAccessReview."""

    @pytest.fixture()
    def no_access_client(
        self,
        gateway_endpoint: str,
        auth_token: str,
        no_access_namespace: str,
        ca_cert: str | None,
        insecure: bool,
    ) -> DataConnectClient:
        if not no_access_namespace:
            pytest.skip("DCH_NO_ACCESS_NAMESPACE not set")
        client = DataConnectClient(
            gateway_endpoint,
            token=auth_token,
            tenant_id=no_access_namespace,
            ca_cert=ca_cert,
            insecure=insecure,
            max_retries=0,
            rest_timeout=10.0,
        )
        yield client
        client.close()

    def test_rest_returns_403(self, no_access_client: DataConnectClient) -> None:
        with pytest.raises(DCHForbiddenError):
            no_access_client.list_connections()

    def test_flight_returns_permission_denied(self, no_access_client: DataConnectClient) -> None:
        with pytest.raises(DCHConnectionError, match=r"(?i)unauthorized|permission.?denied"):
            no_access_client.server_info()


# ---------------------------------------------------------------------------
# Connections are isolated between tenants
# ---------------------------------------------------------------------------


class TestConnectionIsolation:
    """A tenant connection is invisible from the neighbor tenant, for the same caller."""

    def test_list_excludes_other_tenant_connections(
        self, rest_client: DataConnectClient, neighbor_client: DataConnectClient, tenant_connection: DataConnection
    ) -> None:
        assert tenant_connection.id in {c.id for c in rest_client.list_connections()}
        assert tenant_connection.id not in {c.id for c in neighbor_client.list_connections()}

    def test_get_returns_404(self, neighbor_client: DataConnectClient, tenant_connection: DataConnection) -> None:
        with pytest.raises(DCHNotFoundError):
            neighbor_client.get_connection(tenant_connection.id)

    def test_update_returns_404_and_leaves_connection_unchanged(
        self, rest_client: DataConnectClient, neighbor_client: DataConnectClient, tenant_connection: DataConnection
    ) -> None:
        with pytest.raises(DCHNotFoundError):
            neighbor_client.update_connection(tenant_connection.id, properties={"owner": "neighbor"})
        assert rest_client.get_connection(tenant_connection.id).properties["owner"] == "tenant"

    def test_delete_returns_404_and_keeps_connection(
        self, rest_client: DataConnectClient, neighbor_client: DataConnectClient, tenant_connection: DataConnection
    ) -> None:
        with pytest.raises(DCHNotFoundError):
            neighbor_client.delete_connection(tenant_connection.id)
        assert rest_client.get_connection(tenant_connection.id).id == tenant_connection.id

    def test_readiness_rejects_unknown_connection(
        self, neighbor_client: DataConnectClient, tenant_connection: DataConnection
    ) -> None:
        with pytest.raises(DCHValidationError) as exc_info:
            neighbor_client.check_connection_readiness(tenant_connection.id)
        assert exc_info.value.status_code == 400
        assert "invalid_data_connection_id" in exc_info.value.body

    def test_export_returns_404(self, neighbor_client: DataConnectClient, tenant_connection: DataConnection) -> None:
        with pytest.raises(DCHNotFoundError) as exc_info:
            neighbor_client.export_connection(tenant_connection.id, f"dch-e2e-tenancy-{uuid.uuid4().hex[:8]}")
        # "not_found" means the connection lookup failed; a "secret_not_found"
        # 404 would mean the lookup leaked across tenants and only the secret
        # lookup in the neighbor namespace failed.
        assert json.loads(exc_info.value.body)["code"] == "not_found"

    def test_binary_download_returns_404(
        self, neighbor_client: DataConnectClient, tenant_connection: DataConnection
    ) -> None:
        with pytest.raises(DCHNotFoundError):
            b"".join(neighbor_client.download_binary(tenant_connection.id, "any/path.bin"))

    def test_flight_returns_not_found(
        self, neighbor_client: DataConnectClient, tenant_connection: DataConnection
    ) -> None:
        # Flight maps gRPC NOT_FOUND to ArrowKeyError.
        with pytest.raises(pa.ArrowKeyError, match="not found"):
            _check_data_connection(neighbor_client, tenant_connection.id)

    def test_same_name_allowed_in_each_tenant(
        self,
        rest_client: DataConnectClient,
        neighbor_tenant_id: str,
        tenant_id: str,
        create_connection,
        create_neighbor_connection,
    ) -> None:
        global_type = _global_type(rest_client)
        name = f"dch-e2e-tenancy-{uuid.uuid4().hex[:8]}"
        ours = create_connection(
            name=name, connection_type_id=global_type.id, credentials_ref=CredentialsRef(secret=UNUSED_SECRET)
        )
        theirs = create_neighbor_connection(
            name=name, connection_type_id=global_type.id, credentials_ref=CredentialsRef(secret=UNUSED_SECRET)
        )
        assert ours.id != theirs.id
        assert ours.tenant_id == tenant_id
        assert theirs.tenant_id == neighbor_tenant_id


# ---------------------------------------------------------------------------
# Connection types: tenant-scoped vs global
# ---------------------------------------------------------------------------


class TestConnectionTypeIsolation:
    """Tenant-scoped types stay in their tenant; global types are shared read-only."""

    def test_tenant_type_hidden_from_other_tenant(
        self, neighbor_client: DataConnectClient, tenant_type: ConnectionType
    ) -> None:
        assert tenant_type.id not in {ct.id for ct in neighbor_client.list_connection_types()}
        with pytest.raises(DCHNotFoundError):
            neighbor_client.get_connection_type(tenant_type.id)

    def test_tenant_type_not_modifiable_from_other_tenant(
        self, rest_client: DataConnectClient, neighbor_client: DataConnectClient, tenant_type: ConnectionType
    ) -> None:
        with pytest.raises(DCHNotFoundError):
            neighbor_client.update_connection_type(tenant_type.id, description="changed by neighbor")
        with pytest.raises(DCHNotFoundError):
            neighbor_client.delete_connection_type(tenant_type.id)
        assert rest_client.get_connection_type(tenant_type.id).description == tenant_type.description

    def test_create_connection_with_other_tenant_type_is_rejected(
        self, neighbor_client: DataConnectClient, tenant_type: ConnectionType
    ) -> None:
        with pytest.raises(DCHValidationError) as exc_info:
            neighbor_client.create_connection(
                name=f"dch-e2e-tenancy-{uuid.uuid4().hex[:8]}",
                connection_type_id=tenant_type.id,
                data_format="tabular",
                credentials_ref=CredentialsRef(secret=UNUSED_SECRET),
            )
        assert exc_info.value.status_code == 422

    def test_update_connection_to_other_tenant_type_is_rejected(
        self,
        neighbor_client: DataConnectClient,
        create_neighbor_connection,
        tenant_type: ConnectionType,
    ) -> None:
        conn = create_neighbor_connection(
            connection_type_id=_global_type(neighbor_client).id,
            credentials_ref=CredentialsRef(secret=UNUSED_SECRET),
        )
        with pytest.raises(DCHValidationError) as exc_info:
            neighbor_client.update_connection(conn.id, connection_type_id=tenant_type.id)
        assert exc_info.value.status_code == 422

    def test_global_type_visible_in_every_tenant(
        self, rest_client: DataConnectClient, neighbor_client: DataConnectClient
    ) -> None:
        ours = _global_type(rest_client)
        theirs = _global_type(neighbor_client)
        assert ours.id == theirs.id
        assert neighbor_client.get_connection_type(ours.id).id == ours.id

    def test_global_type_read_only_outside_global_tenant(
        self, neighbor_client: DataConnectClient, create_neighbor_connection
    ) -> None:
        global_type = _global_type(neighbor_client)
        # The update writes the current description back, so a missing tenant
        # filter would show up as a 200 without altering the shared type.
        with pytest.raises(DCHNotFoundError):
            neighbor_client.update_connection_type(global_type.id, description=global_type.description)
        # A connection referencing the type makes the delete fail with 409 even
        # if the tenant filter were missing, so the shared type is never lost.
        create_neighbor_connection(
            connection_type_id=global_type.id, credentials_ref=CredentialsRef(secret=UNUSED_SECRET)
        )
        try:
            neighbor_client.delete_connection_type(global_type.id)
        except DCHNotFoundError:
            pass
        except DCHConflictError:
            pytest.fail("delete reached the reference check: global type is not scoped to the global tenant")
        else:
            pytest.fail("neighbor tenant deleted a global connection type")
        assert _global_type(neighbor_client).id == global_type.id


# ---------------------------------------------------------------------------
# Secrets never cross namespaces
# ---------------------------------------------------------------------------


class TestSecretIsolation:
    """Secrets are resolved in, and written to, the request tenant's namespace only."""

    def test_inline_credentials_secret_created_in_request_tenant(
        self,
        neighbor_client: DataConnectClient,
        create_neighbor_connection,
        tenant_id: str,
        neighbor_tenant_id: str,
    ) -> None:
        credentials = InlineCredentials(
            secret=f"dch-e2e-tenancy-{uuid.uuid4().hex[:8]}",
            properties={"URI": "postgresql://e2e-user:e2e-password@localhost:5432/e2e"},
        )
        conn = create_neighbor_connection(connection_type_id=_global_type(neighbor_client).id, credentials=credentials)
        assert conn.tenant_id == neighbor_tenant_id
        assert _secret_exists(neighbor_tenant_id, credentials.secret)
        assert not _secret_exists(tenant_id, credentials.secret)

    def test_secret_ref_resolved_only_in_request_tenant(
        self,
        rest_client: DataConnectClient,
        neighbor_client: DataConnectClient,
        pg_flight_connection: str,
        pg_secret: str,
        create_neighbor_connection_type,
        create_neighbor_connection,
        neighbor_tenant_id: str,
    ) -> None:
        """A neighbor connection naming a secret that exists only in the tenant must not resolve it."""
        assert not _secret_exists(neighbor_tenant_id, pg_secret), "test needs the PG secret absent from the neighbor"

        # Control: the same secret works for the tenant's own connection.
        rest_client.check_connection_readiness(pg_flight_connection)
        _check_data_connection(rest_client, pg_flight_connection)

        neighbor_type = create_neighbor_connection_type(provider="postgres", credentials_fields=_uri_field())
        conn = create_neighbor_connection(
            connection_type_id=neighbor_type.id, credentials_ref=CredentialsRef(secret=pg_secret)
        )

        with pytest.raises(DCHValidationError) as exc_info:
            neighbor_client.check_connection_readiness(conn.id)
        assert "invalid_secret" in exc_info.value.body
        assert neighbor_client.get_connection(conn.id).status.message == "Secret cannot be read"

        # Flight reports the failed secret lookup as UNAVAILABLE.
        with pytest.raises(flight.FlightUnavailableError):
            _check_data_connection(neighbor_client, conn.id)
