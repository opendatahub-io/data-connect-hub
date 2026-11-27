"""Verify that non-monitoring namespaces cannot scrape service metrics."""

from __future__ import annotations

import os
import subprocess

import pytest


@pytest.fixture(scope="module")
def unauthorized_metrics_targets(service_namespace: str) -> tuple[str, list[str]]:
    if os.environ.get("DCH_METRICS_FROM_POD", "").lower() not in ("1", "true", "yes"):
        pytest.skip("metrics policy test requires in-cluster scraping")
    if not service_namespace:
        pytest.skip("DCH_SERVICE_NAMESPACE not set")

    pod_result = subprocess.run(
        [
            "kubectl",
            "get",
            "pods",
            "--namespace",
            service_namespace,
            "--selector=app.kubernetes.io/name=rest-service",
            "--output=jsonpath={.items[0].metadata.name}",
        ],
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
    )
    pod = pod_result.stdout.strip()
    if not pod:
        pytest.fail(f"no REST pod found in {service_namespace!r}")

    rest_result = subprocess.run(
        [
            "kubectl",
            "get",
            "service",
            "--namespace",
            service_namespace,
            "--selector=app.kubernetes.io/name=rest-service",
            "--output=jsonpath={.items[0].metadata.name}",
        ],
        check=True,
        capture_output=True,
        text=True,
        timeout=10,
    )
    rest_service = rest_result.stdout.strip()
    if not rest_service:
        pytest.fail(f"no REST service found in {service_namespace!r}")

    flight_service = os.environ.get("DCH_FLIGHT_SERVICE_NAME")
    if not flight_service:
        services_result = subprocess.run(
            [
                "kubectl",
                "get",
                "service",
                "--namespace",
                service_namespace,
                '--output=jsonpath={range .items[*]}{.metadata.name}{"\\n"}{end}',
            ],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        flight_services = [name for name in services_result.stdout.splitlines() if name.endswith("-flight")]
        if len(flight_services) != 1:
            pytest.fail(f"could not uniquely identify the Flight service in {service_namespace!r}")
        flight_service = flight_services[0]

    return pod, [rest_service, flight_service]


@pytest.mark.parametrize("target_index", [0, 1], ids=["rest", "flight"])
def test_metrics_policy_denies_unauthorized_namespace(
    service_namespace: str,
    unauthorized_metrics_targets: tuple[str, list[str]],
    target_index: int,
) -> None:
    pod, services = unauthorized_metrics_targets
    result = subprocess.run(
        [
            "kubectl",
            "exec",
            "--namespace",
            service_namespace,
            pod,
            "--container=rest-server",
            "--",
            "curl",
            "--fail",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "3",
            "--max-time",
            "5",
            f"http://{services[target_index]}:9090/metrics",
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode in (7, 28), (
        f"metrics access from {service_namespace!r} unexpectedly returned exit code "
        f"{result.returncode}: {result.stderr}"
    )
