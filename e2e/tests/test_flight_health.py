"""gRPC health checks through the configured gateway endpoint."""

from __future__ import annotations

import ipaddress
import os
import ssl
import tempfile
from pathlib import Path
from urllib.parse import urlparse

import grpc
import pytest
from grpc_health.v1 import health_pb2, health_pb2_grpc


def _parse_target(endpoint: str) -> tuple[str, int]:
    parsed = urlparse(endpoint if "://" in endpoint else f"//{endpoint}", scheme="https")
    if not parsed.hostname:
        raise ValueError(f"Invalid DCH_GATEWAY_ENDPOINT: {endpoint!r}")
    return parsed.hostname, parsed.port or 443


def _host_is_loopback(host: str) -> bool:
    if host in {"localhost", "127.0.0.1", "::1"}:
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def _authority_from_server_cert(cert_pem: str) -> str | None:
    with tempfile.NamedTemporaryFile("w", delete=False) as tmp:
        tmp.write(cert_pem)
        cert_path = tmp.name
    try:
        decoded = ssl._ssl._test_decode_cert(cert_path)  # type: ignore[attr-defined]
    finally:
        Path(cert_path).unlink(missing_ok=True)

    for entry_type, entry_value in decoded.get("subjectAltName", []):
        if entry_type == "DNS":
            return entry_value
    return None


def _build_channel(gateway_endpoint: str, insecure: bool, ca_cert: str | None) -> grpc.Channel:
    host, port = _parse_target(gateway_endpoint)
    target = f"{host}:{port}"
    options: list[tuple[str, str]] = []

    if ca_cert:
        root_certs = Path(ca_cert).read_bytes()
    elif insecure:
        # Trust the presented cert directly for self-signed local setups.
        cert_pem = ssl.get_server_certificate((host, port))
        root_certs = cert_pem.encode("utf-8")

        override = os.environ.get("DCH_GRPC_HEALTH_AUTHORITY")
        if not override and _host_is_loopback(host):
            override = _authority_from_server_cert(cert_pem)
        if override:
            options.extend(
                [
                    ("grpc.ssl_target_name_override", override),
                    ("grpc.default_authority", override),
                ]
            )
    else:
        root_certs = None

    creds = grpc.ssl_channel_credentials(root_certificates=root_certs)
    return grpc.secure_channel(target, creds, options=options)


class TestFlightHealth:
    def test_health_check_serving(
        self,
        gateway_endpoint: str,
        insecure: bool,
        ca_cert: str | None,
        gateway_auth_required: bool,
        auth_token: str,
    ) -> None:
        channel = _build_channel(gateway_endpoint, insecure, ca_cert)
        stub = health_pb2_grpc.HealthStub(channel)
        metadata = (("authorization", f"Bearer {auth_token}"),) if gateway_auth_required else None

        try:
            response = stub.Check(health_pb2.HealthCheckRequest(service=""), metadata=metadata, timeout=10)
        except grpc.RpcError as exc:
            if exc.code() == grpc.StatusCode.UNIMPLEMENTED:
                pytest.fail(
                    "gRPC health endpoint is not routed by the Gateway "
                    "(HTTPRoute must include /grpc.health.v1.Health path)."
                )
            raise
        finally:
            channel.close()

        assert response.status == health_pb2.HealthCheckResponse.SERVING
