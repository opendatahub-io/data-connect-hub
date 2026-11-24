"""Legacy connection import: ConfigMap → connection type, Secret → connection.

The dc-controller watches ConfigMaps labelled opendatahub.io/connection-type
and Secrets labelled opendatahub.io/dashboard, then imports them into the REST
API as connection types and connections. These tests verify the end-to-end flow
in the CI Kind cluster where the controller, REST service, and the legacy K8s
resources all coexist.

The ConfigMap and Secret fixtures are created by run-e2e.sh before pytest
starts. The tests wait (with polling) for the controller to complete the
import — the controller reconcile loop may need a few seconds after the
resource is created.
"""

from __future__ import annotations

import time

import pytest

from data_connect_hub import DataConnectClient

LEGACY_S3_CM_NAME = "s3"
LEGACY_S3_SECRET_NAME = "e2e-legacy-s3-conn"
LEGACY_OCI_CM_NAME = "oci-v1"
LEGACY_OCI_SECRET_NAME = "e2e-legacy-oci-conn"
URI_V1_CM_NAME = "uri-v1"
URI_V1_SECRET_NAME = "e2e-legacy-uri-conn"

# ODH Dashboard ConfigMap name -> DCH built-in connection type name that covers it.
BUILTIN_COVERED = {
    LEGACY_S3_CM_NAME: "S3",
    LEGACY_OCI_CM_NAME: "OCI",
    URI_V1_CM_NAME: "URI",
}

POLL_TIMEOUT = 60
POLL_INTERVAL = 2


def _poll_connection(client: DataConnectClient, name: str, timeout: int = POLL_TIMEOUT):
    """Wait for a connection with the given name to appear."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for conn in client.list_connections():
            if conn.name == name:
                return conn
        time.sleep(POLL_INTERVAL)
    return None


@pytest.fixture(scope="module")
def legacy_import_enabled():
    """Skip the entire module when legacy import fixtures were not created."""
    import os

    if os.environ.get("DCH_LEGACY_IMPORT_ENABLED", "").lower() not in ("true", "1", "yes"):
        pytest.skip("DCH_LEGACY_IMPORT_ENABLED not set (legacy import fixtures not created)")


class TestLegacyConnectionTypeImport:
    """Verify that ODH Dashboard connection-type ConfigMaps covered by a
    built-in connection type are skipped."""

    def test_dashboard_configmaps_not_imported_as_duplicates(
        self, legacy_import_enabled, rest_client: DataConnectClient
    ) -> None:
        """The s3, oci-v1, and uri-v1 ConfigMaps should NOT create separate
        connection types because built-in types already cover them."""
        time.sleep(10)
        types = rest_client.list_connection_types()
        names = [ct.name for ct in types]
        for cm_name, builtin_name in BUILTIN_COVERED.items():
            assert cm_name not in names, f"{cm_name} should not be imported as a separate connection type"
            assert names.count(builtin_name) == 1, f"built-in {builtin_name} connection type should exist"


class TestLegacyConnectionImport:
    """Verify that ODH connection Secrets are imported and resolve to the
    built-in connection types."""

    @pytest.mark.parametrize(
        ("display_name", "secret_name", "builtin_name"),
        [
            ("E2E Legacy S3 Connection", LEGACY_S3_SECRET_NAME, BUILTIN_COVERED[LEGACY_S3_CM_NAME]),
            ("E2E Legacy OCI Connection", LEGACY_OCI_SECRET_NAME, BUILTIN_COVERED[LEGACY_OCI_CM_NAME]),
            ("E2E Legacy URI Connection", URI_V1_SECRET_NAME, BUILTIN_COVERED[URI_V1_CM_NAME]),
        ],
    )
    def test_secret_imported_via_builtin_alias(
        self,
        legacy_import_enabled,
        rest_client: DataConnectClient,
        display_name: str,
        secret_name: str,
        builtin_name: str,
    ) -> None:
        """A Secret referencing a Dashboard connection type should resolve to
        the built-in type via the builtin provider alias mechanism."""
        conn = _poll_connection(rest_client, display_name)
        assert conn is not None, (
            f"connection {display_name!r} not found after {POLL_TIMEOUT}s — "
            "controller may not have resolved the builtin alias"
        )
        assert conn.credentials_ref is not None
        assert conn.credentials_ref.secret == secret_name

        types = rest_client.list_connection_types()
        builtin = next((ct for ct in types if ct.name == builtin_name), None)
        assert builtin is not None
        assert conn.data_connection_type_id == builtin.id, (
            f"connection should reference the built-in {builtin_name} type"
        )

    def test_s3_secret_description_imported(self, legacy_import_enabled, rest_client: DataConnectClient) -> None:
        conn = _poll_connection(rest_client, "E2E Legacy S3 Connection")
        assert conn is not None
        assert conn.description == "E2E legacy S3 description"
