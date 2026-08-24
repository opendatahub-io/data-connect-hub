"""Flight metrics tests: verify Prometheus metrics are exposed and populated.

Self-contained: calls server_info() to generate sql_info counters,
then verifies the metrics endpoint reflects those calls.
Skips if DCH_FLIGHT_METRICS_URL is not set.
"""

from __future__ import annotations

import re

import httpx
import pytest

from data_connect_hub import DataConnectClient


@pytest.fixture(scope="module")
def metrics_body(
    flight_url: str,
    flight_metrics_url: str | None,
    auth_token: str,
    tenant_id: str,
    insecure: bool,
) -> str:
    if not flight_metrics_url:
        pytest.skip("DCH_FLIGHT_METRICS_URL not set")

    client = DataConnectClient(flight_url=flight_url, token=auth_token, tenant_id=tenant_id, insecure=insecure)
    client.server_info()

    r = httpx.get(f"{flight_metrics_url}/metrics", timeout=10.0)
    assert r.status_code == 200, f"metrics endpoint returned {r.status_code}"
    return r.text


def _metric_value(body: str, name: str, labels: dict[str, str]) -> float | None:
    for line in body.splitlines():
        if not line.startswith(name + "{"):
            continue
        if all(f'{k}="{v}"' in line for k, v in labels.items()):
            match = re.search(r"\}\s+(\S+)$", line)
            if match:
                return float(match.group(1))
    return None


class TestFlightMetrics:
    def test_requests_total_exists(self, metrics_body: str) -> None:
        assert "dch_flight_rpc_requests_total" in metrics_body

    def test_duration_metric_exists(self, metrics_body: str) -> None:
        assert any(
            line.startswith("dch_flight_rpc_duration_seconds")
            for line in metrics_body.splitlines()
            if not line.startswith("#")
        )

    def test_get_flight_info_sql_info_ok(self, metrics_body: str) -> None:
        val = _metric_value(
            metrics_body,
            "dch_flight_rpc_requests_total",
            {"method": "arrow.flight.protocol.FlightService/GetFlightInfo", "operation": "sql_info", "status": "OK"},
        )
        assert val is not None and val > 0, f"GetFlightInfo/sql_info OK count should be > 0, got {val}"
