"""Global namespace tests: a labeled namespace grants the Auth CR groups access to its DCH resources.

run-e2e.sh sets ``DCH_GLOBAL_NAMESPACE`` only when the platform Auth CR exists,
and grants the e2e user ``dch-read-write`` there as a writer. The module labels
the namespace, checks what dc-controller sets up, and removes the label again.

Two identities take part:

- writer: the e2e user, through its own RoleBinding in the global namespace.
- consumer: the denied SA, which holds no RoleBinding anywhere. It reaches the
  global namespace only through the Auth CR ``allowedGroups``, so its tests run
  only when those include ``system:authenticated``.

Connection types in the global namespace stay scoped to it: the label only adds
RoleBindings and never changes the global connection type tenants, so it does
not roll the REST or Flight Deployments. The module waits for dc-controller to
create or remove the RoleBindings after each label change.
"""

from __future__ import annotations

import base64
import contextlib
import json
import subprocess
import time
import uuid
from collections.abc import Callable

import pytest

from data_connect_hub import (
    ConnectionType,
    CredentialsRef,
    DataConnectClient,
    DCHConnectionError,
    DCHForbiddenError,
    DCHServerError,
    DCHTimeoutError,
    DCHValidationError,
    InlineCredentials,
)

GLOBAL_LABEL = "opendatahub.io/global-data-connect-hub-project"
MANAGED_SELECTOR = "dataconnecthub.opendatahub.io/global-namespace=true"
WRITER_ROLE_BINDING = "dch-e2e-global-writer"
ROLE_BINDINGS = {
    "dch-global-read": "dch-read",
    "dch-global-read-write": "dch-read-write",
    "dch-global-rest-service-secret-access": "dch-rest-service-secret-access",
    "dch-global-flight-service-secret-access": "dch-flight-service-secret-access",
}
BUILT_IN_POSTGRES_TYPE = "Postgres"
ALL_AUTHENTICATED = "system:authenticated"
KUBECTL_TIMEOUT_SECONDS = 60
WAIT_TIMEOUT_SECONDS = 180.0

# Errors the Gateway returns while a REST or Flight pod is being replaced.
TRANSIENT_ERRORS = (DCHConnectionError, DCHServerError, DCHTimeoutError)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _kubectl(*args: str) -> str:
    result = subprocess.run(
        ["kubectl", *args], check=False, capture_output=True, text=True, timeout=KUBECTL_TIMEOUT_SECONDS
    )
    if result.returncode != 0:
        pytest.fail(f"kubectl {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


def _kubectl_json(*args: str) -> dict:
    return json.loads(_kubectl(*args, "--output=json"))


def _wait_for(check: Callable[[], bool], message: str, timeout: float = WAIT_TIMEOUT_SECONDS) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(2)
    pytest.fail(message)


def _managed_role_bindings(namespace: str) -> dict[str, dict]:
    items = _kubectl_json("get", "rolebindings", "--namespace", namespace, "--selector", MANAGED_SELECTOR)["items"]
    return {rb["metadata"]["name"]: rb for rb in items}


def _set_global_label(namespace: str, labeled: bool) -> None:
    """Add or remove the global label and wait for dc-controller to reconcile the RoleBindings."""
    labels = _kubectl_json("get", "namespace", namespace)["metadata"].get("labels", {})
    if (GLOBAL_LABEL in labels) == labeled:
        return
    change = f"{GLOBAL_LABEL}={namespace}" if labeled else f"{GLOBAL_LABEL}-"
    _kubectl("label", "namespace", namespace, change, "--overwrite")
    if labeled:
        _wait_for(
            lambda: bool(_managed_role_bindings(namespace)),
            f"dc-controller did not create RoleBindings in {namespace!r}; "
            "was the platform Auth CRD installed before dc-controller started?",
        )
    else:
        _wait_for(
            lambda: not _managed_role_bindings(namespace),
            f"dc-controller did not remove its RoleBindings from {namespace!r}",
        )


def _role_binding_exists(namespace: str, name: str) -> bool:
    out = _kubectl("get", "rolebinding", name, "--namespace", namespace, "--ignore-not-found", "--output=name")
    return bool(out.strip())


def _secret_exists(namespace: str, name: str) -> bool:
    out = _kubectl("get", "secret", name, "--namespace", namespace, "--ignore-not-found", "--output=name")
    return bool(out.strip())


def _secret_properties(namespace: str, name: str) -> dict[str, str]:
    data = _kubectl_json("get", "secret", name, "--namespace", namespace).get("data", {})
    return {key: base64.b64decode(value).decode() for key, value in data.items()}


def _can_list_connection_types(client: DataConnectClient) -> bool:
    """Report whether *client* may list connection types; the services cache authorization decisions."""
    try:
        client.list_connection_types()
    except (DCHForbiddenError, *TRANSIENT_ERRORS):
        return False
    return True


def _client(endpoint: str, token: str, tenant: str, ca_cert: str | None, insecure: bool) -> DataConnectClient:
    return DataConnectClient(
        endpoint,
        token=token,
        tenant_id=tenant,
        ca_cert=ca_cert,
        insecure=insecure,
        max_retries=3,
        rest_timeout=15.0,
    )


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def global_namespace(global_namespace_id: str) -> str:
    """The labeled global namespace; the label is removed after the module."""
    if not global_namespace_id:
        pytest.skip("DCH_GLOBAL_NAMESPACE not set (run-e2e.sh found no platform Auth CR)")
    _set_global_label(global_namespace_id, True)
    yield global_namespace_id  # type: ignore[misc]
    _set_global_label(global_namespace_id, False)


@pytest.fixture(scope="module")
def auth_groups() -> tuple[list[str], list[str]]:
    """The platform Auth CR ``allowedGroups`` and ``adminGroups``."""
    spec = _kubectl_json("get", "auths.services.platform.opendatahub.io", "auth").get("spec", {})
    return spec.get("allowedGroups", []), spec.get("adminGroups", [])


@pytest.fixture(scope="module")
def writer_client(
    global_namespace: str, gateway_endpoint: str, auth_token: str, ca_cert: str | None, insecure: bool
) -> DataConnectClient:
    client = _client(gateway_endpoint, auth_token, global_namespace, ca_cert, insecure)
    yield client  # type: ignore[misc]
    client.close()


@pytest.fixture(scope="module")
def tenant_client(
    global_namespace: str, gateway_endpoint: str, auth_token: str, tenant_id: str, ca_cert: str | None, insecure: bool
) -> DataConnectClient:
    client = _client(gateway_endpoint, auth_token, tenant_id, ca_cert, insecure)
    yield client  # type: ignore[misc]
    client.close()


@pytest.fixture(scope="module")
def consumer_client(
    global_namespace: str,
    auth_groups: tuple[list[str], list[str]],
    gateway_endpoint: str,
    persona_denied_auth_token: str,
    ca_cert: str | None,
    insecure: bool,
) -> DataConnectClient:
    allowed, _ = auth_groups
    if ALL_AUTHENTICATED not in allowed:
        pytest.skip(f"Auth CR allowedGroups does not include {ALL_AUTHENTICATED}")
    if not persona_denied_auth_token:
        pytest.skip("DCH_PERSONA_DENIED_AUTH_TOKEN not set")
    client = _client(gateway_endpoint, persona_denied_auth_token, global_namespace, ca_cert, insecure)
    _wait_for(
        lambda: _can_list_connection_types(client),
        f"the denied user cannot read {global_namespace!r} through the Auth CR allowedGroups",
    )
    yield client  # type: ignore[misc]
    client.close()


@pytest.fixture(scope="module")
def global_type(writer_client: DataConnectClient) -> ConnectionType:
    """A connection type the writer creates in the global namespace."""
    ct = writer_client.create_connection_type(
        name=f"e2e-global-ct-{uuid.uuid4().hex[:8]}",
        provider="postgres",
        description="e2e global namespace connection type",
    )
    yield ct  # type: ignore[misc]
    with contextlib.suppress(Exception):
        writer_client.delete_connection_type(ct.id)


# ---------------------------------------------------------------------------
# Tests (run in order: the last one removes the label)
# ---------------------------------------------------------------------------


class TestGlobalNamespace:
    def test_role_bindings_follow_auth_groups(
        self,
        global_namespace: str,
        auth_groups: tuple[list[str], list[str]],
        service_namespace: str,
        rest_service_account: str,
        flight_service_account: str,
    ) -> None:
        _wait_for(
            lambda: set(_managed_role_bindings(global_namespace)) == set(ROLE_BINDINGS),
            f"dc-controller did not create {sorted(ROLE_BINDINGS)} in {global_namespace!r}",
        )
        bindings = _managed_role_bindings(global_namespace)
        for name, role in ROLE_BINDINGS.items():
            assert bindings[name]["roleRef"]["name"] == role, name

        def subjects(name: str) -> list[tuple[str, str, str]]:
            return sorted((s["kind"], s.get("namespace", ""), s["name"]) for s in bindings[name].get("subjects", []))

        allowed, admin = auth_groups
        assert subjects("dch-global-read") == sorted(("Group", "", g) for g in set(allowed) | set(admin))
        assert subjects("dch-global-read-write") == sorted(("Group", "", g) for g in set(admin))
        assert subjects("dch-global-rest-service-secret-access") == [
            ("ServiceAccount", service_namespace, rest_service_account)
        ]
        assert subjects("dch-global-flight-service-secret-access") == [
            ("ServiceAccount", service_namespace, flight_service_account)
        ]

    def test_connection_type_stays_in_global_namespace(
        self, global_type: ConnectionType, tenant_client: DataConnectClient, global_namespace: str
    ) -> None:
        """A connection in another tenant must never reference a global namespace type.

        Otherwise designating a different global namespace would leave that
        connection unusable.
        """
        assert global_type.tenant_id == global_namespace
        assert global_type.id not in [ct.id for ct in tenant_client.list_connection_types()]
        with pytest.raises(DCHValidationError) as exc_info:
            tenant_client.create_connection(
                name=f"e2e-global-cross-{uuid.uuid4().hex[:8]}",
                connection_type_id=global_type.id,
                data_format="tabular",
                credentials_ref=CredentialsRef(secret="dch-e2e-global-unused"),  # notsecret
            )
        assert exc_info.value.status_code == 422

    def test_consumer_reads_but_cannot_write(
        self, consumer_client: DataConnectClient, global_type: ConnectionType
    ) -> None:
        assert global_type.id in [ct.id for ct in consumer_client.list_connection_types()]
        with pytest.raises(DCHForbiddenError):
            consumer_client.create_connection_type(
                name=f"e2e-global-denied-{uuid.uuid4().hex[:8]}", provider="postgres"
            )

    def test_consumer_ingests_through_global_connection(
        self,
        consumer_client: DataConnectClient,
        writer_client: DataConnectClient,
        global_namespace: str,
        tenant_id: str,
        pg_secret: str | None,
    ) -> None:
        """Inline credentials make REST write the Secret and Flight read it, both through the managed bindings."""
        if not pg_secret:
            pytest.skip("DCH_PG_SECRET not set (run e2e/run-e2e.sh first)")
        postgres = next(
            (
                ct
                for ct in writer_client.list_connection_types()
                if ct.name == BUILT_IN_POSTGRES_TYPE and not ct.tenant_id
            ),
            None,
        )
        assert postgres is not None, f"built-in connection type {BUILT_IN_POSTGRES_TYPE!r} not found"

        credentials = InlineCredentials(
            secret=f"dch-e2e-global-{uuid.uuid4().hex[:8]}",
            properties=_secret_properties(tenant_id, pg_secret),
        )
        conn = writer_client.create_connection(
            name=f"e2e-global-conn-{uuid.uuid4().hex[:8]}",
            connection_type_id=postgres.id,
            data_format="tabular",
            credentials=credentials,
            properties={},
        )
        try:
            assert _secret_exists(global_namespace, credentials.secret)
            assert conn.id in [c.id for c in consumer_client.list_connections()]
            table = consumer_client.read("SELECT name FROM dch_e2e.cities ORDER BY id", conn.id)
            assert table.column("name").to_pylist() == ["Tokyo", "London", "Paris"]
        finally:
            with contextlib.suppress(Exception):
                writer_client.delete_connection(conn.id)
            _kubectl("delete", "secret", credentials.secret, "--namespace", global_namespace, "--ignore-not-found")

    def test_removing_the_label_removes_role_bindings(self, global_namespace: str) -> None:
        # run-e2e.sh creates the writer binding unless DCH_AUTH_TOKEN is supplied.
        writer_bound = _role_binding_exists(global_namespace, WRITER_ROLE_BINDING)
        _set_global_label(global_namespace, False)
        if writer_bound:
            assert _role_binding_exists(global_namespace, WRITER_ROLE_BINDING), (
                "administrator-created bindings must survive"
            )
