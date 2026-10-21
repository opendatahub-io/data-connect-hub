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

POLL_TIMEOUT = 60
POLL_INTERVAL = 2


def _poll_connection_type(client: DataConnectClient, name: str, timeout: int = POLL_TIMEOUT):
    """Wait for a connection type with the given name to appear."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for ct in client.list_connection_types():
            if ct.name == name:
                return ct
        time.sleep(POLL_INTERVAL)
    return None


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
    """Verify that ODH connection-type ConfigMaps are imported."""

    def test_s3_configmap_imported_as_connection_type(
        self, legacy_import_enabled, rest_client: DataConnectClient
    ) -> None:
        """The s3 ConfigMap (as shipped by ODH Dashboard) should be imported
        as a connection type with matching name and fields."""
        ct = _poll_connection_type(rest_client, LEGACY_S3_CM_NAME)
        assert ct is not None, (
            f"connection type {LEGACY_S3_CM_NAME!r} not found after {POLL_TIMEOUT}s — "
            "controller may not have reconciled the ConfigMap"
        )
        assert ct.provider == LEGACY_S3_CM_NAME
        field_names = {f.name for f in ct.credentials_fields}
        assert "AWS_ACCESS_KEY_ID" in field_names
        assert "AWS_SECRET_ACCESS_KEY" in field_names
        assert "AWS_S3_ENDPOINT" in field_names

    def test_oci_v1_configmap_imported_as_connection_type(
        self, legacy_import_enabled, rest_client: DataConnectClient
    ) -> None:
        """The oci-v1 ConfigMap (as shipped by ODH Dashboard) should be
        imported as a connection type."""
        ct = _poll_connection_type(rest_client, LEGACY_OCI_CM_NAME)
        assert ct is not None, (
            f"connection type {LEGACY_OCI_CM_NAME!r} not found after {POLL_TIMEOUT}s — "
            "controller may not have reconciled the ConfigMap"
        )
        assert ct.provider == LEGACY_OCI_CM_NAME
        field_names = {f.name for f in ct.credentials_fields}
        assert "OCI_HOST" in field_names

    def test_uri_v1_configmap_not_imported_as_duplicate(
        self, legacy_import_enabled, rest_client: DataConnectClient
    ) -> None:
        """The uri-v1 ConfigMap should NOT create a separate connection type
        because the built-in URI type already covers this provider."""
        time.sleep(10)
        types = rest_client.list_connection_types()
        uri_v1_types = [ct for ct in types if ct.name == URI_V1_CM_NAME]
        assert len(uri_v1_types) == 0, (
            f"uri-v1 should not be imported as a separate connection type, "
            f"but found: {uri_v1_types}"
        )
        uri_types = [ct for ct in types if ct.name == "URI"]
        assert len(uri_types) == 1, "built-in URI connection type should exist"


class TestLegacyConnectionImport:
    """Verify that ODH connection Secrets are imported."""

    def test_s3_secret_imported_as_connection(
        self, legacy_import_enabled, rest_client: DataConnectClient
    ) -> None:
        """A Secret with opendatahub.io/dashboard label and
        connection-type-ref=s3 should be imported as a connection."""
        conn = _poll_connection(rest_client, "E2E Legacy S3 Connection")
        assert conn is not None, (
            f"connection 'E2E Legacy S3 Connection' not found after {POLL_TIMEOUT}s — "
            "controller may not have reconciled the Secret"
        )
        assert conn.credentials_ref is not None
        assert conn.credentials_ref.secret == LEGACY_S3_SECRET_NAME

    def test_oci_v1_secret_imported_as_connection(
        self, legacy_import_enabled, rest_client: DataConnectClient
    ) -> None:
        """A Secret with connection-type-ref=oci-v1 should be imported as a
        connection referencing the imported oci-v1 type."""
        conn = _poll_connection(rest_client, "E2E Legacy OCI Connection")
        assert conn is not None, (
            f"connection 'E2E Legacy OCI Connection' not found after {POLL_TIMEOUT}s — "
            "controller may not have reconciled the Secret"
        )
        assert conn.credentials_ref is not None
        assert conn.credentials_ref.secret == LEGACY_OCI_SECRET_NAME

    def test_uri_v1_secret_imported_via_builtin_alias(
        self, legacy_import_enabled, rest_client: DataConnectClient
    ) -> None:
        """A Secret referencing uri-v1 should resolve to the built-in URI type
        via the builtin provider alias mechanism."""
        conn = _poll_connection(rest_client, "E2E Legacy URI Connection")
        assert conn is not None, (
            f"connection 'E2E Legacy URI Connection' not found after {POLL_TIMEOUT}s — "
            "controller may not have resolved uri-v1 alias"
        )
        assert conn.credentials_ref is not None
        assert conn.credentials_ref.secret == URI_V1_SECRET_NAME

        types = rest_client.list_connection_types()
        uri_type = next((ct for ct in types if ct.name == "URI"), None)
        assert uri_type is not None
        assert conn.data_connection_type_id == uri_type.id, (
            "connection should reference the built-in URI type, not a uri-v1 type"
        )
