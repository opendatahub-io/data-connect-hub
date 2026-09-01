"""E2E test fixtures for Data Connect Hub.

Configuration is read from environment variables, typically written
by setup.sh into .env and loaded at the top of this file.

    DCH_GATEWAY_ENDPOINT   Gateway host or host:port (REST + Flight)
    DCH_TENANT_ID          Tenant namespace
    DCH_AUTH_TOKEN         Bearer token
    DCH_INSECURE           Skip TLS verify           (default: false)
    DCH_CA_CERT            CA cert path              (optional)
    DCH_PG_SECRET          K8s secret name for PG    (set by setup.sh, enables query tests)
"""

from __future__ import annotations

import contextlib
import os
import uuid
from pathlib import Path

import httpx
import pytest

from data_connect_hub import CredentialsRef, DataConnectClient
from data_connect_hub.client import _build_urls

# ---------------------------------------------------------------------------
# Load .env written by setup.sh (does not override existing env vars)
# ---------------------------------------------------------------------------

_ENV_FILE = Path(__file__).parent / ".env"
if _ENV_FILE.exists():
    for line in _ENV_FILE.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        os.environ.setdefault(key, value)


# ---------------------------------------------------------------------------
# Config fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def gateway_endpoint() -> str:
    return os.environ.get("DCH_GATEWAY_ENDPOINT", "https://localhost:8443")


@pytest.fixture(scope="session")
def rest_url(gateway_endpoint: str) -> str:
    """REST base URL the SDK derives from *gateway_endpoint*.

    Only for the raw ``httpx`` fixture below, so it targets the exact URL the
    SDK targets; SDK-based tests should take ``gateway_endpoint`` directly.
    """
    rest, _ = _build_urls(gateway_endpoint)
    return rest


@pytest.fixture(scope="session")
def flight_metrics_url() -> str | None:
    return os.environ.get("DCH_FLIGHT_METRICS_URL") or None


@pytest.fixture(scope="session")
def tenant_id() -> str:
    return os.environ.get("DCH_TENANT_ID", "e2e-test")


@pytest.fixture(scope="session")
def auth_token() -> str:
    return os.environ.get("DCH_AUTH_TOKEN", "")


@pytest.fixture(scope="session")
def denied_auth_token() -> str:
    return os.environ.get("DCH_DENIED_AUTH_TOKEN", "")


@pytest.fixture(scope="session")
def no_access_namespace() -> str:
    return os.environ.get("DCH_NO_ACCESS_NAMESPACE", "")


@pytest.fixture(scope="session")
def ca_cert() -> str | None:
    return os.environ.get("DCH_CA_CERT")


@pytest.fixture(scope="session")
def insecure() -> bool:
    return os.environ.get("DCH_INSECURE", "false").lower() in ("true", "1", "yes")


@pytest.fixture(scope="session")
def pg_secret() -> str | None:
    return os.environ.get("DCH_PG_SECRET") or None


@pytest.fixture(scope="session")
def s3_secret() -> str | None:
    return os.environ.get("DCH_S3_SECRET") or None


@pytest.fixture(scope="session")
def s3_csv_query() -> str | None:
    return os.environ.get("DCH_S3_CSV_QUERY") or None


@pytest.fixture(scope="session")
def s3_parquet_query() -> str | None:
    return os.environ.get("DCH_S3_PARQUET_QUERY") or None


@pytest.fixture(scope="session")
def s3_jsonl_query() -> str | None:
    return os.environ.get("DCH_S3_JSONL_QUERY") or None


@pytest.fixture(scope="session")
def s3_binary_path() -> str | None:
    return os.environ.get("DCH_S3_BINARY_PATH") or None


@pytest.fixture(scope="session")
def milvus_secret() -> str | None:
    return os.environ.get("DCH_MILVUS_SECRET") or None


@pytest.fixture(scope="session")
def es_secret() -> str | None:
    return os.environ.get("DCH_ES_SECRET") or None


@pytest.fixture(scope="session")
def es_apikey_secret() -> str | None:
    return os.environ.get("DCH_ES_APIKEY_SECRET") or None


@pytest.fixture(scope="session")
def neo4j_secret() -> str | None:
    return os.environ.get("DCH_NEO4J_SECRET") or None


@pytest.fixture(scope="session")
def uri_secret() -> str | None:
    return os.environ.get("DCH_URI_SECRET") or None


# ---------------------------------------------------------------------------
# Client fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def dch_client(
    gateway_endpoint: str,
    tenant_id: str,
    auth_token: str,
    ca_cert: str | None,
    insecure: bool,
) -> DataConnectClient:
    client = DataConnectClient(
        gateway_endpoint,
        token=auth_token,
        tenant_id=tenant_id,
        ca_cert=ca_cert,
        insecure=insecure,
        max_retries=1,
        rest_timeout=15.0,
    )
    yield client  # type: ignore[misc]
    client.close()


@pytest.fixture(scope="session")
def rest_client(dch_client: DataConnectClient) -> DataConnectClient:
    """Alias of ``dch_client``, kept for readability in REST-only tests."""
    return dch_client


@pytest.fixture(scope="session")
def http_client(rest_url: str, ca_cert: str | None, insecure: bool) -> httpx.Client:
    if insecure:
        verify: str | bool = False
    elif ca_cert:
        verify = ca_cert
    else:
        verify = True
    client = httpx.Client(base_url=rest_url, timeout=10.0, verify=verify)
    yield client  # type: ignore[misc]
    client.close()


# ---------------------------------------------------------------------------
# Factory fixtures (auto-cleanup)
# ---------------------------------------------------------------------------


def _unique_name(prefix: str) -> str:
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


@pytest.fixture()
def create_connection_type(rest_client: DataConnectClient):
    """Factory: creates connection types, deletes them after the test."""
    created_ids: list[str] = []

    def _factory(
        *,
        name: str | None = None,
        provider: str = "postgres",
        description: str | None = "e2e test connection type",
    ):
        ct = rest_client.create_connection_type(
            name=name or _unique_name("e2e-ct"),
            provider=provider,
            description=description,
        )
        created_ids.append(ct.id)
        return ct

    yield _factory

    for ct_id in reversed(created_ids):
        with contextlib.suppress(Exception):
            rest_client.delete_connection_type(ct_id)


@pytest.fixture()
def create_connection(rest_client: DataConnectClient):
    """Factory: creates connections, deletes them after the test."""
    created_ids: list[str] = []

    def _factory(
        *,
        name: str | None = None,
        connection_type_id: str,
        data_format: str = "tabular",
        credentials_ref=None,
        properties: dict[str, str] | None = None,
    ):
        conn = rest_client.create_connection(
            name=name or _unique_name("e2e-conn"),
            connection_type_id=connection_type_id,
            data_format=data_format,
            credentials_ref=credentials_ref,
            properties=properties,
        )
        created_ids.append(conn.id)
        return conn

    yield _factory

    for conn_id in reversed(created_ids):
        with contextlib.suppress(Exception):
            rest_client.delete_connection(conn_id)


# ---------------------------------------------------------------------------
# Flight query fixture (module-scoped)
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def flight_client_factory(
    gateway_endpoint: str,
    auth_token: str,
    tenant_id: str,
    ca_cert: str | None,
    insecure: bool,
):
    """Factory that creates a raw pyarrow.flight client with headers for a given connection."""
    import pyarrow.flight as flight
    from data_connect_hub.client import _build_urls

    _, flight_url = _build_urls(gateway_endpoint)

    def _factory(connection_id: str) -> tuple[flight.FlightClient, flight.FlightCallOptions]:
        kwargs: dict = {}
        if insecure:
            kwargs["disable_server_verification"] = True
        if ca_cert:
            kwargs["tls_root_certs"] = Path(ca_cert).read_bytes()

        client = flight.connect(flight_url, **kwargs)
        headers = [
            (b"x-data-connection-id", connection_id.encode()),
            (b"x-tenant-id", tenant_id.encode()),
            (b"authorization", f"Bearer {auth_token}".encode()),
        ]
        opts = flight.FlightCallOptions(headers=headers)
        return client, opts

    return _factory


@pytest.fixture(scope="module")
def pg_flight_connection(
    pg_secret: str | None,
    rest_client: DataConnectClient,
) -> str:
    """Create connection type + connection for Flight SQL query tests.

    The K8s secret and test data are prepared by setup.sh.
    Returns the connection ID. Cleans up REST resources after the module.
    """
    if not pg_secret:
        pytest.skip("DCH_PG_SECRET not set (run e2e/setup.sh first)")

    ct = rest_client.create_connection_type(
        name=_unique_name("e2e-pg-type"),
        provider="postgres",
        description="e2e query test",
    )
    conn = rest_client.create_connection(
        name=_unique_name("e2e-pg-conn"),
        connection_type_id=ct.id,
        data_format="tabular",
        credentials_ref=CredentialsRef(secret=pg_secret),
        properties={},
    )

    yield conn.id

    with contextlib.suppress(Exception):
        rest_client.delete_connection(conn.id)
    with contextlib.suppress(Exception):
        rest_client.delete_connection_type(ct.id)


@pytest.fixture(scope="module")
def s3_flight_connection(
    s3_secret: str | None,
    rest_client: DataConnectClient,
) -> str:
    """Create connection type + connection for Flight S3 query tests.

    The K8s secret is prepared by run-e2e.sh. S3 data must already exist.
    Returns the connection ID. Cleans up REST resources after the module.
    """
    if not s3_secret:
        pytest.skip("DCH_S3_SECRET not set (set AWS credentials in env file)")

    ct = rest_client.create_connection_type(
        name=_unique_name("e2e-s3-type"),
        provider="s3",
        description="e2e S3 test",
    )
    conn = rest_client.create_connection(
        name=_unique_name("e2e-s3-conn"),
        connection_type_id=ct.id,
        data_format="tabular",
        credentials_ref=CredentialsRef(secret=s3_secret),
        properties={},
    )

    yield conn.id

    with contextlib.suppress(Exception):
        rest_client.delete_connection(conn.id)
    with contextlib.suppress(Exception):
        rest_client.delete_connection_type(ct.id)


@pytest.fixture(scope="module")
def milvus_flight_connection(
    milvus_secret: str | None,
    rest_client: DataConnectClient,
) -> str:
    """Create connection type + connection for Flight Milvus query tests.

    The K8s secret is prepared by run-e2e.sh. Milvus data must already exist.
    Returns the connection ID. Cleans up REST resources after the module.
    """
    if not milvus_secret:
        pytest.skip("DCH_MILVUS_SECRET not set (set DCH_MILVUS_HOST in env file)")

    ct = rest_client.create_connection_type(
        name=_unique_name("e2e-milvus-type"),
        provider="milvus",
        description="e2e Milvus test",
    )
    conn = rest_client.create_connection(
        name=_unique_name("e2e-milvus-conn"),
        connection_type_id=ct.id,
        data_format="tabular",
        credentials_ref=CredentialsRef(secret=milvus_secret),
        properties={},
    )

    yield conn.id

    with contextlib.suppress(Exception):
        rest_client.delete_connection(conn.id)
    with contextlib.suppress(Exception):
        rest_client.delete_connection_type(ct.id)


@pytest.fixture(scope="module")
def es_flight_connection(
    es_secret: str | None,
    rest_client: DataConnectClient,
) -> str:
    """Create connection type + connection for Flight Elasticsearch query tests.

    The K8s secret is prepared by run-e2e.sh. ES data must already exist.
    Returns the connection ID. Cleans up REST resources after the module.
    """
    if not es_secret:
        pytest.skip("DCH_ES_SECRET not set (set DCH_ES_URI in env file)")

    ct = rest_client.create_connection_type(
        name=_unique_name("e2e-es-type"),
        provider="elasticsearch",
        description="e2e Elasticsearch test",
    )
    conn = rest_client.create_connection(
        name=_unique_name("e2e-es-conn"),
        connection_type_id=ct.id,
        data_format="tabular",
        credentials_ref=CredentialsRef(secret=es_secret),
        properties={},
    )

    yield conn.id

    with contextlib.suppress(Exception):
        rest_client.delete_connection(conn.id)
    with contextlib.suppress(Exception):
        rest_client.delete_connection_type(ct.id)


@pytest.fixture(scope="module")
def es_apikey_flight_connection(
    es_apikey_secret: str | None,
    rest_client: DataConnectClient,
) -> str:
    """Create connection for Flight Elasticsearch API key auth tests.

    Uses an API key instead of basic auth. Skips if API key secret is not available.
    """
    if not es_apikey_secret:
        pytest.skip("DCH_ES_APIKEY_SECRET not set (ES API key not created)")

    ct = rest_client.create_connection_type(
        name=_unique_name("e2e-es-apikey-type"),
        provider="elasticsearch",
        description="e2e Elasticsearch API key test",
    )
    conn = rest_client.create_connection(
        name=_unique_name("e2e-es-apikey-conn"),
        connection_type_id=ct.id,
        data_format="tabular",
        credentials_ref=CredentialsRef(secret=es_apikey_secret),
        properties={},
    )

    yield conn.id

    with contextlib.suppress(Exception):
        rest_client.delete_connection(conn.id)
    with contextlib.suppress(Exception):
        rest_client.delete_connection_type(ct.id)


@pytest.fixture(scope="module")
def neo4j_flight_connection(
    neo4j_secret: str | None,
    rest_client: DataConnectClient,
) -> str:
    """Create connection type + connection for Flight Neo4j query tests.

    The K8s secret is prepared by run-e2e.sh. Neo4j data must already exist.
    Returns the connection ID. Cleans up REST resources after the module.
    """
    if not neo4j_secret:
        pytest.skip("DCH_NEO4J_SECRET not set (set DCH_NEO4J_URI in env file)")

    ct = rest_client.create_connection_type(
        name=_unique_name("e2e-neo4j-type"),
        provider="neo4j",
        description="e2e Neo4j test",
    )
    conn = rest_client.create_connection(
        name=_unique_name("e2e-neo4j-conn"),
        connection_type_id=ct.id,
        data_format="tabular",
        credentials_ref=CredentialsRef(secret=neo4j_secret),
        properties={},
    )

    yield conn.id

    with contextlib.suppress(Exception):
        rest_client.delete_connection(conn.id)
    with contextlib.suppress(Exception):
        rest_client.delete_connection_type(ct.id)


@pytest.fixture(scope="module")
def uri_flight_connection(
    uri_secret: str | None,
    rest_client: DataConnectClient,
) -> str:
    """Create connection type + connection for Flight URI query tests.

    The K8s secret and test HTTP server are prepared by run-e2e.sh.
    Returns the connection ID. Cleans up REST resources after the module.
    """
    if not uri_secret:
        pytest.skip("DCH_URI_SECRET not set (set DCH_TENANT_URI in env file)")

    ct = rest_client.create_connection_type(
        name=_unique_name("e2e-uri-type"),
        provider="uri",
        description="e2e URI test",
    )
    conn = rest_client.create_connection(
        name=_unique_name("e2e-uri-conn"),
        connection_type_id=ct.id,
        data_format="tabular",
        credentials_ref=CredentialsRef(secret=uri_secret),
        properties={},
    )

    yield conn.id

    with contextlib.suppress(Exception):
        rest_client.delete_connection(conn.id)
    with contextlib.suppress(Exception):
        rest_client.delete_connection_type(ct.id)
