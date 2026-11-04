"""Global namespace tests: a labeled namespace shares DCH resources with every tenant.

run-e2e.sh sets ``DCH_GLOBAL_NAMESPACE`` only when the platform Auth CR exists,
and grants the e2e user ``dch-read-write`` there as a curator. The module labels
the namespace, checks what dc-controller sets up, and removes the label again.

Two identities take part:

- curator: the e2e user, through its own RoleBinding in the global namespace.
- consumer: the denied SA, which holds no RoleBinding anywhere. It reaches the
  global namespace only through the Auth CR ``allowedGroups``, so its tests run
  only when those include ``system:authenticated``.

Changing the label changes the global connection type tenants in the REST and
Flight ConfigMaps, which rolls both Deployments. The module waits for each
rollout before calling DCH, and again after removing the label, so later modules
never run during a rollout.
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
    DataConnectClient,
    DCHConnectionError,
    DCHForbiddenError,
    DCHServerError,
    DCHTimeoutError,
    InlineCredentials,
)

GLOBAL_LABEL = "opendatahub.io/global-data-registry-project"
MANAGED_SELECTOR = "dataconnecthub.opendatahub.io/global-namespace=true"
SERVICE_SELECTOR = "app.kubernetes.io/name in (rest-service,flight-service)"
CURATOR_ROLE_BINDING = "dch-e2e-global-curator"
ROLE_BINDINGS = {
    "dch-global-read": "dch-read",
    "dch-global-read-write": "dch-read-write",
    "dch-global-rest-service-secret-access": "dch-rest-service-secret-access",
    "dch-global-flight-service-secret-access": "dch-flight-service-secret-access",
}
BUILT_IN_POSTGRES_TYPE = "Postgres"
ALL_AUTHENTICATED = "system:authenticated"
ROLLOUT_TIMEOUT_SECONDS = 300
WAIT_TIMEOUT_SECONDS = 180.0

# Errors the Gateway returns while a REST or Flight pod is being replaced.
TRANSIENT_ERRORS = (DCHConnectionError, DCHServerError, DCHTimeoutError)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _kubectl(*args: str) -> str:
    result = subprocess.run(
        ["kubectl", *args], check=False, capture_output=True, text=True, timeout=ROLLOUT_TIMEOUT_SECONDS
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


def _service_generations(service_namespace: str) -> dict[str, int]:
    deployments = _kubectl_json("get", "deployments", "--namespace", service_namespace, "--selector", SERVICE_SELECTOR)
    return {d["metadata"]["name"]: d["metadata"]["generation"] for d in deployments["items"]}


def _set_global_label(namespace: str, service_namespace: str, labeled: bool) -> None:
    """Add or remove the global label and wait for REST and Flight to roll out."""
    labels = _kubectl_json("get", "namespace", namespace)["metadata"].get("labels", {})
    if (GLOBAL_LABEL in labels) == labeled:
        return
    before = _service_generations(service_namespace)
    assert before, f"no REST or Flight Deployment found in {service_namespace!r}"

    change = f"{GLOBAL_LABEL}={namespace}" if labeled else f"{GLOBAL_LABEL}-"
    _kubectl("label", "namespace", namespace, change, "--overwrite")

    # dc-controller renders the new tenants into the ConfigMaps, which changes
    # the config hash on both Deployments. Wait for that before checking the
    # rollout, or the check could pass against the old generation.
    _wait_for(
        lambda: all(gen > before.get(name, 0) for name, gen in _service_generations(service_namespace).items()),
        "dc-controller did not roll out REST and Flight after the global label changed; "
        "was the platform Auth CRD installed before dc-controller started?",
    )
    _kubectl(
        "rollout",
        "status",
        "deployment",
        "--namespace",
        service_namespace,
        "--selector",
        SERVICE_SELECTOR,
        f"--timeout={ROLLOUT_TIMEOUT_SECONDS}s",
    )


def _managed_role_bindings(namespace: str) -> dict[str, dict]:
    items = _kubectl_json("get", "rolebindings", "--namespace", namespace, "--selector", MANAGED_SELECTOR)["items"]
    return {rb["metadata"]["name"]: rb for rb in items}


def _role_binding_exists(namespace: str, name: str) -> bool:
    out = _kubectl("get", "rolebinding", name, "--namespace", namespace, "--ignore-not-found", "--output=name")
    return bool(out.strip())


def _secret_exists(namespace: str, name: str) -> bool:
    out = _kubectl("get", "secret", name, "--namespace", namespace, "--ignore-not-found", "--output=name")
    return bool(out.strip())


def _secret_properties(namespace: str, name: str) -> dict[str, str]:
    data = _kubectl_json("get", "secret", name, "--namespace", namespace).get("data", {})
    return {key: base64.b64decode(value).decode() for key, value in data.items()}


def _type_listed(client: DataConnectClient, name: str) -> bool | None:
    """Report whether *client*'s listing contains *name*; None when the Gateway is still settling."""
    try:
        return any(ct.name == name for ct in client.list_connection_types())
    except TRANSIENT_ERRORS:
        return None


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
def global_namespace(global_namespace_id: str, service_namespace: str) -> str:
    """The labeled global namespace; the label is removed after the module."""
    if not global_namespace_id:
        pytest.skip("DCH_GLOBAL_NAMESPACE not set (run-e2e.sh found no platform Auth CR)")
    _set_global_label(global_namespace_id, service_namespace, True)
    yield global_namespace_id  # type: ignore[misc]
    _set_global_label(global_namespace_id, service_namespace, False)


@pytest.fixture(scope="module")
def auth_groups() -> tuple[list[str], list[str]]:
    """The platform Auth CR ``allowedGroups`` and ``adminGroups``."""
    spec = _kubectl_json("get", "auths.services.platform.opendatahub.io", "auth").get("spec", {})
    return spec.get("allowedGroups", []), spec.get("adminGroups", [])


# The clients below are created after the label rollout, so they never reuse a
# connection to a replaced pod.


@pytest.fixture(scope="module")
def curator_client(
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
    yield client  # type: ignore[misc]
    client.close()


@pytest.fixture(scope="module")
def global_type(curator_client: DataConnectClient) -> ConnectionType:
    """A connection type the curator creates in the global namespace."""
    ct = curator_client.create_connection_type(
        name=f"e2e-global-ct-{uuid.uuid4().hex[:8]}",
        provider="postgres",
        description="e2e global namespace connection type",
    )
    yield ct  # type: ignore[misc]
    with contextlib.suppress(Exception):
        curator_client.delete_connection_type(ct.id)


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

    def test_connection_type_visible_to_every_tenant(
        self, global_type: ConnectionType, tenant_client: DataConnectClient
    ) -> None:
        _wait_for(
            lambda: _type_listed(tenant_client, global_type.name) is True,
            f"global connection type {global_type.name!r} is not visible to the tenant",
        )
        listed = next(ct for ct in tenant_client.list_connection_types() if ct.name == global_type.name)
        assert listed.id == global_type.id
        # Global types come back without a tenant_id.
        assert not listed.tenant_id

    def test_consumer_reads_but_cannot_curate(
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
        curator_client: DataConnectClient,
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
                for ct in curator_client.list_connection_types()
                if ct.name == BUILT_IN_POSTGRES_TYPE and not ct.tenant_id
            ),
            None,
        )
        assert postgres is not None, f"built-in connection type {BUILT_IN_POSTGRES_TYPE!r} not found"

        credentials = InlineCredentials(
            secret=f"dch-e2e-global-{uuid.uuid4().hex[:8]}",
            properties=_secret_properties(tenant_id, pg_secret),
        )
        conn = curator_client.create_connection(
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
                curator_client.delete_connection(conn.id)
            _kubectl("delete", "secret", credentials.secret, "--namespace", global_namespace, "--ignore-not-found")

    def test_removing_the_label_revokes_access(
        self,
        global_namespace: str,
        service_namespace: str,
        global_type: ConnectionType,
        tenant_client: DataConnectClient,
    ) -> None:
        # run-e2e.sh creates the curator binding unless DCH_AUTH_TOKEN is supplied.
        curator_bound = _role_binding_exists(global_namespace, CURATOR_ROLE_BINDING)
        _set_global_label(global_namespace, service_namespace, False)

        _wait_for(
            lambda: not _managed_role_bindings(global_namespace),
            f"dc-controller did not remove its RoleBindings from {global_namespace!r}",
        )
        if curator_bound:
            assert _role_binding_exists(global_namespace, CURATOR_ROLE_BINDING), (
                "administrator-created bindings must survive"
            )
        _wait_for(
            lambda: _type_listed(tenant_client, global_type.name) is False,
            f"connection type {global_type.name!r} is still visible to the tenant after the label was removed",
        )
