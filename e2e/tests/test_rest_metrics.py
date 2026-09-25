"""REST metrics tests: verify Prometheus metrics are exposed and populated."""

from __future__ import annotations

import re

import httpx
import pytest


@pytest.fixture(scope="module")
def rest_metrics_body(http_client: httpx.Client, rest_metrics_url: str | None) -> str:
    if not rest_metrics_url:
        pytest.skip("DCH_REST_METRICS_URL not set")

    response = http_client.get("/health")
    assert response.status_code == 200

    response = httpx.get(f"{rest_metrics_url}/metrics", timeout=10.0)
    assert response.status_code == 200, f"metrics endpoint returned {response.status_code}"
    return response.text


def _metric_value(body: str, name: str, labels: dict[str, str]) -> float | None:
    for line in body.splitlines():
        if not line.startswith(name + "{"):
            continue
        if all(f'{key}="{value}"' in line for key, value in labels.items()):
            match = re.search(r"\}\s+(\S+)$", line)
            if match:
                return float(match.group(1))
    return None


class TestRestMetrics:
    def test_requests_total_exists(self, rest_metrics_body: str) -> None:
        assert "dch_rest_requests_total" in rest_metrics_body

    def test_duration_metric_exists(self, rest_metrics_body: str) -> None:
        assert any(
            line.startswith("dch_rest_request_duration_seconds")
            for line in rest_metrics_body.splitlines()
            if not line.startswith("#")
        )

    def test_health_request_recorded(self, rest_metrics_body: str) -> None:
        value = _metric_value(
            rest_metrics_body,
            "dch_rest_requests_total",
            {"method": "GET", "route": "/health", "status": "200"},
        )
        assert value is not None and value > 0, f"GET /health 200 count should be > 0, got {value}"

    def test_active_requests_metric_present(self, rest_metrics_body: str) -> None:
        assert any(line.startswith("# TYPE dch_rest_requests_active gauge") for line in rest_metrics_body.splitlines())
