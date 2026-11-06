"""REST audit logging against a live audit-enabled DCH deployment."""

from __future__ import annotations

import time
import uuid
from collections.abc import Callable
from datetime import UTC, datetime

import httpx
import pyarrow.flight as flight
import pytest
from data_connect_hub import DataConnectClient, DCHNotFoundError, DCHQueryError

_LOG_POLL_TIMEOUT = 15
_LOG_POLL_INTERVAL = 1


@pytest.fixture(autouse=True)
def require_audit_enabled(audit_enabled: bool) -> None:
    if not audit_enabled:
        pytest.skip("DCH_AUDIT_ENABLED is not true")


def _wait_for_record(
    pod_logs_since: Callable[[str, datetime], list[dict]],
    service: str,
    since: datetime,
    **expected: object,
) -> dict:
    deadline = time.monotonic() + _LOG_POLL_TIMEOUT
    matches: list[dict] = []
    while time.monotonic() < deadline:
        records = pod_logs_since(service, since)
        matches = [record for record in records if all(record.get(key) == value for key, value in expected.items())]
        if matches:
            return matches[-1]
        time.sleep(_LOG_POLL_INTERVAL)
    pytest.fail(f"audit record not found; expected fields: {expected}")


def test_rest_success_audit_record(
    rest_client: DataConnectClient,
    tenant_id: str,
    jaeger_query_url: str | None,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    started = datetime.now(UTC)

    rest_client.list_connection_types()

    record = _wait_for_record(
        pod_logs_since,
        "rest-service",
        started,
        logger="audit",
        status_code=200,
        **{
            "audit.resource": "connection_type",
            "audit.action": "list",
            "audit.tenant_id": tenant_id,
        },
    )
    assert uuid.UUID(record["request_id"]).version == 4
    assert record["audit.actor"] not in ("", "anonymous")
    assert record["duration_ms"] >= 0
    assert "level" not in record
    assert "method" not in record
    assert "route" not in record
    assert "outcome" not in record
    assert "msg" not in record
    assert "error_code" not in record
    assert "error_message" not in record
    if jaeger_query_url:
        assert record["trace_id"]
        assert record["span_id"]


def test_rest_failure_audit_record(
    rest_client: DataConnectClient,
    tenant_id: str,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    missing_id = str(uuid.uuid4())
    started = datetime.now(UTC)

    with pytest.raises(DCHNotFoundError):
        rest_client.get_connection(missing_id)

    record = _wait_for_record(
        pod_logs_since,
        "rest-service",
        started,
        logger="audit",
        resource_id=missing_id,
        status_code=404,
        **{
            "audit.resource": "connection",
            "audit.action": "get",
            "audit.tenant_id": tenant_id,
        },
    )
    assert record["error_code"] == "not_found"
    assert record["error_message"] == "Not Found"


def test_rest_get_audit_record_contains_resource_id(
    rest_client: DataConnectClient,
    create_connection_type,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    connection_type = create_connection_type(provider="postgres")
    started = datetime.now(UTC)

    rest_client.get_connection_type(connection_type.id)

    record = _wait_for_record(
        pod_logs_since,
        "rest-service",
        started,
        logger="audit",
        resource_id=connection_type.id,
        status_code=200,
        **{
            "audit.resource": "connection_type",
            "audit.action": "get",
        },
    )
    assert record["resource_id"] == connection_type.id


def test_health_check_is_not_audited(
    http_client: httpx.Client,
    gateway_auth_required: bool,
    auth_token: str,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    started = datetime.now(UTC)
    headers = {"Authorization": f"Bearer {auth_token}"} if gateway_auth_required else None

    response = http_client.get("/health", headers=headers)
    assert response.status_code == 200
    time.sleep(_LOG_POLL_INTERVAL)

    records = pod_logs_since("rest-service", started)
    health_audits = [
        record
        for record in records
        if record.get("logger") == "audit" and record.get("request-path") == "/health"
    ]
    assert health_audits == []


def test_flight_query_audit_record(
    dch_client: DataConnectClient,
    pg_flight_connection: str,
    tenant_id: str,
    jaeger_query_url: str | None,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    started = datetime.now(UTC)

    dch_client.read("SELECT 1 AS audit_test", pg_flight_connection)

    record = _wait_for_record(
        pod_logs_since,
        "flight-service",
        started,
        logger="audit",
        resource_id=pg_flight_connection,
        grpc_status="OK",
        **{
            "audit.resource": "connection",
            "audit.action": "query",
            "audit.tenant_id": tenant_id,
        },
    )
    assert uuid.UUID(record["request_id"]).version == 4
    assert record["audit.actor"] not in ("", "unknown")
    assert record["caller_principal"] == record["audit.actor"]
    assert record["request-path"] == "/arrow.flight.protocol.FlightService/DoGet"
    assert record["duration_ms"] >= 0
    assert "parent_request_id" not in record
    assert "error_code" not in record
    assert "error_message" not in record
    assert "level" not in record
    assert "msg" not in record
    assert "method" not in record
    assert "route" not in record
    assert "outcome" not in record
    if jaeger_query_url:
        assert record["trace_id"]
        assert record["span_id"]


def test_rest_readiness_links_flight_audit_record(
    rest_client: DataConnectClient,
    pg_flight_connection: str,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    started = datetime.now(UTC)

    rest_client.check_connection_readiness(pg_flight_connection)

    rest_record = _wait_for_record(
        pod_logs_since,
        "rest-service",
        started,
        logger="audit",
        resource_id=pg_flight_connection,
        status_code=204,
        **{
            "audit.resource": "connection",
            "audit.action": "check_readiness",
        },
    )
    flight_record = _wait_for_record(
        pod_logs_since,
        "flight-service",
        started,
        logger="audit",
        resource_id=pg_flight_connection,
        grpc_status="OK",
        **{
            "audit.resource": "connection",
            "audit.action": "check_readiness",
        },
    )
    assert flight_record["parent_request_id"] == rest_record["request_id"]
    assert flight_record["audit.actor"] == rest_record["audit.actor"]
    assert flight_record["caller_principal"] != flight_record["audit.actor"]


def test_rest_binary_download_links_flight_audit_record(
    rest_client: DataConnectClient,
    uri_flight_connection: str,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    started = datetime.now(UTC)

    data = b"".join(rest_client.download_binary(uri_flight_connection, "api/binary.dat"))
    assert data == b"binary-test-data-for-e2e\n"

    rest_record = _wait_for_record(
        pod_logs_since,
        "rest-service",
        started,
        logger="audit",
        resource_id=uri_flight_connection,
        status_code=200,
        **{
            "audit.resource": "connection",
            "audit.action": "download",
        },
    )
    flight_record = _wait_for_record(
        pod_logs_since,
        "flight-service",
        started,
        logger="audit",
        resource_id=uri_flight_connection,
        grpc_status="OK",
        **{
            "audit.resource": "connection",
            "audit.action": "download_binary",
        },
    )
    assert flight_record["parent_request_id"] == rest_record["request_id"]
    assert flight_record["audit.actor"] == rest_record["audit.actor"]
    assert flight_record["caller_principal"] != flight_record["audit.actor"]


def test_flight_query_failure_audit_is_sanitized(
    dch_client: DataConnectClient,
    pg_flight_connection: str,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    started = datetime.now(UTC)

    with pytest.raises(DCHQueryError):
        dch_client.read(
            "SELECT 1 / (id - id) AS audit_failure FROM dch_e2e.cities",
            pg_flight_connection,
        )

    record = _wait_for_record(
        pod_logs_since,
        "flight-service",
        started,
        logger="audit",
        resource_id=pg_flight_connection,
        **{
            "audit.resource": "connection",
            "audit.action": "query",
        },
    )
    assert record["grpc_status"] != "OK"
    assert record["error_code"] == record["grpc_status"]
    assert record["error_message"] == "gRPC request failed"
    assert "division by zero" not in str(record).lower()


def test_flight_unauthenticated_do_get_audit_record(
    dch_client: DataConnectClient,
    tenant_id: str,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    client = dch_client._require_flight()._flight_connect()
    options = flight.FlightCallOptions(headers=[(b"x-tenant-id", tenant_id.encode())])
    started = datetime.now(UTC)

    try:
        with pytest.raises(flight.FlightUnauthenticatedError):
            client.do_get(flight.Ticket(b"invalid-ticket"), options).read_all()
    finally:
        client.close()

    record = _wait_for_record(
        pod_logs_since,
        "flight-service",
        started,
        logger="audit",
        grpc_status="Unauthenticated",
        **{
            "audit.resource": "flight_server",
            "audit.action": "authenticate",
        },
    )
    assert record["audit.actor"] == "unknown"
    assert record["caller_principal"] == "unknown"
    assert record["error_code"] == "Unauthenticated"
    assert record["error_message"] == "gRPC request failed"


def test_flight_direct_caller_cannot_spoof_forwarded_identity(
    dch_client: DataConnectClient,
    pg_flight_connection: str,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    flight_client = dch_client._require_flight()
    original_headers = dict(flight_client._static_headers)
    spoofed_parent = str(uuid.uuid4())
    flight_client._static_headers.update(
        {
            "x-remote-user": "spoofed-user",
            "x-request-id": spoofed_parent,
        }
    )
    started = datetime.now(UTC)

    try:
        dch_client.read("SELECT 1 AS spoof_test", pg_flight_connection)
    finally:
        flight_client._static_headers = original_headers

    record = _wait_for_record(
        pod_logs_since,
        "flight-service",
        started,
        logger="audit",
        resource_id=pg_flight_connection,
        grpc_status="OK",
        **{
            "audit.resource": "connection",
            "audit.action": "query",
        },
    )
    assert record["audit.actor"] != "spoofed-user"
    assert record["caller_principal"] == record["audit.actor"]
    assert record.get("parent_request_id") != spoofed_parent


def test_flight_health_check_is_not_audited(
    gateway_endpoint: str,
    insecure: bool,
    ca_cert: str | None,
    pod_logs_since: Callable[[str, datetime], list[dict]],
) -> None:
    from grpc_health.v1 import health_pb2, health_pb2_grpc

    from tests.test_flight_health import _build_channel

    channel = _build_channel(gateway_endpoint, insecure, ca_cert)
    stub = health_pb2_grpc.HealthStub(channel)
    started = datetime.now(UTC)

    try:
        response = stub.Check(health_pb2.HealthCheckRequest(service=""), timeout=10)
    finally:
        channel.close()
    assert response.status == health_pb2.HealthCheckResponse.SERVING
    time.sleep(_LOG_POLL_INTERVAL)

    records = pod_logs_since("flight-service", started)
    health_audits = [
        record
        for record in records
        if record.get("logger") == "audit" and str(record.get("request-path", "")).startswith("/grpc.health")
    ]
    assert health_audits == []
