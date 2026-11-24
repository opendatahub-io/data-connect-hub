"""gRPC health checks through the configured gateway endpoint."""

from __future__ import annotations

import http.client
import os
import ssl
import tempfile
import urllib.request
from pathlib import Path
from urllib.parse import urlparse, urlsplit

import grpc
import pytest
from grpc_health.v1 import health_pb2, health_pb2_grpc

from data_connect_hub._flight import _bypasses_proxy, _grpc_proxy_options

_CERT_FETCH_TIMEOUT = 10.0


def _parse_target(endpoint: str) -> tuple[str, int]:
    parsed = urlparse(endpoint if "://" in endpoint else f"//{endpoint}", scheme="https")
    if not parsed.hostname:
        raise ValueError(f"Invalid DCH_GATEWAY_ENDPOINT: {endpoint!r}")
    return parsed.hostname, parsed.port or 443


def _authority_from_server_cert(cert_pem: str) -> str | None:
    with tempfile.NamedTemporaryFile("w", delete=False) as tmp:
        tmp.write(cert_pem)
        cert_path = tmp.name
    try:
        decoded = ssl._ssl._test_decode_cert(cert_path)  # type: ignore[attr-defined]
    finally:
        Path(cert_path).unlink(missing_ok=True)

    for entry_type, entry_value in decoded.get("subjectAltName", []):
        if entry_type == "DNS" and not entry_value.startswith("*"):
            return entry_value
    for entry_type, entry_value in decoded.get("subjectAltName", []):
        if entry_type == "DNS" and entry_value.startswith("*."):
            return "x-grpc-health" + entry_value[1:]
    return None


def _fetch_server_cert(host: str, port: int) -> str:
    """Return the server's PEM certificate, honoring the proxy environment.

    ``ssl.get_server_certificate`` always dials directly, which hangs where
    egress to the Gateway is only allowed through an HTTP proxy. Tunnel
    through the proxy with CONNECT using the same proxy resolution as the SDK.
    """
    proxies = urllib.request.getproxies_environment()
    proxy = proxies.get("https") or proxies.get("http")
    if not proxy or _bypasses_proxy(host, port, proxies.get("no", "")):
        return ssl.get_server_certificate((host, port), timeout=_CERT_FETCH_TIMEOUT)

    parsed = urlsplit(proxy if "://" in proxy else f"http://{proxy}")
    if not parsed.hostname:
        raise ValueError(f"Invalid proxy URL: {proxy!r}")
    conn = http.client.HTTPSConnection(
        parsed.hostname,
        parsed.port or 80,
        timeout=_CERT_FETCH_TIMEOUT,
        context=ssl._create_unverified_context(),
    )
    try:
        conn.set_tunnel(host, port)
        conn.connect()
        der = conn.sock.getpeercert(binary_form=True)
    finally:
        conn.close()
    if not der:
        raise RuntimeError(f"No certificate presented by {host}:{port}")
    return ssl.DER_cert_to_PEM_cert(der)


def _build_channel(gateway_endpoint: str, insecure: bool, ca_cert: str | None) -> grpc.Channel:
    host, port = _parse_target(gateway_endpoint)
    target = f"{host}:{port}"
    # Same proxy resolution as the SDK, so the health check takes the SDK's network path.
    options: list[tuple[str, str | int]] = list(_grpc_proxy_options(f"grpc+tls://{target}"))

    if ca_cert:
        root_certs = Path(ca_cert).read_bytes()
    elif insecure:
        cert_pem = _fetch_server_cert(host, port)
        root_certs = cert_pem.encode("utf-8")

        override = os.environ.get("DCH_GRPC_HEALTH_AUTHORITY")
        # In insecure mode we already trust the exact certificate we just
        # fetched, so the expected authority can be derived from it for any
        # endpoint shape. External gateway addresses serve the router's
        # wildcard certificate for the cluster domain, which never matches
        # the ELB hostname.
        if not override:
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
    def test_health_check_serving(self, gateway_endpoint: str, insecure: bool, ca_cert: str | None) -> None:
        channel = _build_channel(gateway_endpoint, insecure, ca_cert)
        stub = health_pb2_grpc.HealthStub(channel)

        try:
            response = stub.Check(health_pb2.HealthCheckRequest(service=""), timeout=10)
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
