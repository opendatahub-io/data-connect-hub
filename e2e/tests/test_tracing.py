"""Tracing tests: verify OpenTelemetry spans reach Jaeger via the OTEL Collector.

Generates REST and Flight traffic, then queries the Jaeger Query API to confirm
spans are recorded with expected attributes.
Skips if DCH_JAEGER_QUERY_URL is not set.
"""

from __future__ import annotations

import time

import httpx
import pytest
from data_connect_hub import DataConnectClient

_TRACE_POLL_TIMEOUT = 30
_TRACE_POLL_INTERVAL = 2


def _all_spans(traces: list[dict]) -> list[dict]:
    """Flatten traces into a list of spans."""
    return [span for trace in traces for span in trace.get("spans", [])]


def _span_has_tag(span: dict, key: str) -> bool:
    """Check if a span has a tag with the given key."""
    return any(tag["key"] == key for tag in span.get("tags", []))


def _query_traces(jaeger_url: str, service: str, limit: int = 20) -> list[dict]:
    """Query Jaeger for traces from *service*."""
    r = httpx.get(
        f"{jaeger_url}/api/traces",
        params={"service": service, "lookback": "5m", "limit": limit},
        timeout=10.0,
    )
    assert r.status_code == 200, f"Jaeger query returned {r.status_code}: {r.text}"
    return r.json().get("data", [])


def _poll_traces(jaeger_url: str, service: str) -> list[dict]:
    """Poll Jaeger until traces appear or the deadline expires."""
    deadline = time.monotonic() + _TRACE_POLL_TIMEOUT
    traces: list[dict] = []
    while time.monotonic() < deadline:
        traces = _query_traces(jaeger_url, service)
        if _all_spans(traces):
            break
        time.sleep(_TRACE_POLL_INTERVAL)
    return traces


# ---------------------------------------------------------------------------
# REST tracing
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def rest_traces(
    rest_client: DataConnectClient,
    jaeger_query_url: str | None,
) -> list[dict]:
    if not jaeger_query_url:
        pytest.skip("DCH_JAEGER_QUERY_URL not set")

    rest_client.list_connection_types()

    return _poll_traces(jaeger_query_url, "dch-rest-service")


class TestRestTracing:
    def test_rest_traces_exist(self, rest_traces: list[dict]) -> None:
        assert len(rest_traces) > 0, "expected at least one trace from dch-rest-service"

    def test_rest_span_has_http_method(self, rest_traces: list[dict]) -> None:
        spans = _all_spans(rest_traces)
        assert any(_span_has_tag(s, "http.request.method") for s in spans), "no span with http.request.method tag found"

    def test_rest_span_has_route(self, rest_traces: list[dict]) -> None:
        spans = _all_spans(rest_traces)
        assert any(_span_has_tag(s, "url.path") for s in spans), "no span with url.path tag found"


# ---------------------------------------------------------------------------
# Flight tracing
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def flight_traces(
    gateway_endpoint: str,
    auth_token: str,
    tenant_id: str,
    insecure: bool,
    jaeger_query_url: str | None,
) -> list[dict]:
    if not jaeger_query_url:
        pytest.skip("DCH_JAEGER_QUERY_URL not set")

    client = DataConnectClient(
        gateway_endpoint,
        token=auth_token,
        tenant_id=tenant_id,
        insecure=insecure,
    )
    try:
        client.server_info()
    finally:
        client.close()

    return _poll_traces(jaeger_query_url, "dch-flight-service")


class TestFlightTracing:
    def test_flight_traces_exist(self, flight_traces: list[dict]) -> None:
        assert len(flight_traces) > 0, "expected at least one trace from dch-flight-service"

    def test_flight_span_has_rpc_method(self, flight_traces: list[dict]) -> None:
        spans = _all_spans(flight_traces)
        assert any(_span_has_tag(s, "rpc.method") for s in spans), "no span with rpc.method tag found"
