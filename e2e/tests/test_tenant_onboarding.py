"""Tenant onboarding: labelled namespaces get service RBAC automatically.

The dc-controller watches namespaces labelled
``dataconnecthub.opendatahub.io/tenant=true`` and reconciles the
``dch-rest-service-secret-access`` RoleBinding into each one. These tests
reproduce the acceptance criteria of the tenant-RBAC fix: a user with DCH API
access can create connections in a newly created project without any manual
per-namespace service RBAC setup.
"""

from __future__ import annotations

import contextlib
import subprocess
import time
import uuid
from collections.abc import Iterator

import pytest

from data_connect_hub import CredentialField, DataConnectClient, InlineCredentials

TENANT_LABEL = "dataconnecthub.opendatahub.io/tenant"
SECRET_ACCESS_ROLEBINDING = "dch-rest-service-secret-access"
CONTROLLER_RECONCILE_TIMEOUT = 120.0


def _kubectl(*args: str, check: bool = True, timeout: float = 30.0) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["kubectl", *args],
        check=check,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def _wait_for_rolebinding(namespace: str, *, present: bool, timeout: float = CONTROLLER_RECONCILE_TIMEOUT) -> None:
    """Poll until the controller-reconciled RoleBinding matches *present*."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = _kubectl(
            "get",
            "rolebinding",
            SECRET_ACCESS_ROLEBINDING,
            "--namespace",
            namespace,
            "--ignore-not-found",
            check=False,
        )
        if bool(result.stdout.strip()) == present:
            return
        time.sleep(2)
    state = "present" if present else "absent"
    pytest.fail(
        f"RoleBinding {SECRET_ACCESS_ROLEBINDING} in namespace {namespace!r} "
        f"did not become {state} within {timeout:.0f}s"
    )


@pytest.fixture()
def fresh_tenant_namespace() -> Iterator[str]:
    """A newly created namespace labelled as a DCH tenant; deleted after the test."""
    name = f"dch-e2e-onboarding-{uuid.uuid4().hex[:8]}"
    _kubectl("create", "namespace", name)
    try:
        _kubectl("label", "namespace", name, f"{TENANT_LABEL}=true")
        yield name
    finally:
        _kubectl("delete", "namespace", name, "--ignore-not-found", "--timeout=90s", check=False, timeout=100.0)


class TestTenantOnboarding:
    def test_new_labelled_namespace_supports_inline_credentials(
        self,
        gateway_endpoint: str,
        auth_token: str,
        ca_cert: str | None,
        insecure: bool,
        tenant_id: str,
        e2e_user_sa: str,
        fresh_tenant_namespace: str,
    ) -> None:
        """Connection creation works in a new project with no manual service RBAC."""
        if not e2e_user_sa:
            pytest.skip("E2E_USER_SA_NAME not set (bring-your-own-token run)")

        # The controller reconciles the REST service's secret-access
        # RoleBinding from the tenant label alone.
        _wait_for_rolebinding(fresh_tenant_namespace, present=True)

        # Grant the e2e user DCH API access in the new project — the only
        # setup a project admin performs; no service-account RBAC involved.
        _kubectl(
            "create",
            "rolebinding",
            "dch-e2e-onboarding-read-write",
            "--namespace",
            fresh_tenant_namespace,
            "--clusterrole=dch-read-write",
            f"--serviceaccount={tenant_id}:{e2e_user_sa}",
        )

        client = DataConnectClient(
            gateway_endpoint,
            token=auth_token,
            tenant_id=fresh_tenant_namespace,
            ca_cert=ca_cert,
            insecure=insecure,
            max_retries=3,
            rest_timeout=15.0,
        )
        connection_type = None
        connection = None
        try:
            connection_type = client.create_connection_type(
                name=f"e2e-onboarding-type-{uuid.uuid4().hex[:8]}",
                provider="postgres",
                description="e2e tenant onboarding test",
                credentials_fields=[
                    CredentialField(name="URI", label="URI", required=True, type="string"),
                ],
            )
            secret_name = f"e2e-onboarding-{uuid.uuid4().hex[:8]}"
            connection = client.create_connection(
                name="e2e-onboarding-conn",
                connection_type_id=connection_type.id,
                data_format="tabular",
                credentials=InlineCredentials(
                    secret=secret_name,
                    properties={"URI": "postgresql://e2e-user:e2e-password@localhost:5432/e2e"},
                ),
                properties={},
            )
            assert connection.credentials_ref is not None
            assert connection.credentials_ref.secret == secret_name

            # The REST service created the credentials secret in the new
            # namespace, proving the reconciled binding works.
            secret = _kubectl("get", "secret", secret_name, "--namespace", fresh_tenant_namespace)
            assert secret_name in secret.stdout
        finally:
            if connection is not None:
                with contextlib.suppress(Exception):
                    client.delete_connection(connection.id)
            if connection_type is not None:
                with contextlib.suppress(Exception):
                    client.delete_connection_type(connection_type.id)
            client.close()

    def test_removing_the_label_removes_the_rolebinding(self, fresh_tenant_namespace: str) -> None:
        """Unlabelling a namespace withdraws the service's secret access."""
        _wait_for_rolebinding(fresh_tenant_namespace, present=True)

        _kubectl("label", "namespace", fresh_tenant_namespace, f"{TENANT_LABEL}-")

        _wait_for_rolebinding(fresh_tenant_namespace, present=False)
