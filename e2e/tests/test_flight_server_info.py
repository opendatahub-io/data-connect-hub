"""Flight SqlInfo metadata: verify server advertises correct capabilities."""

from __future__ import annotations

from data_connect_hub.flight import FlightSQLClient


class TestFlightServerInfo:
    def _get_info(self, flight_url: str, auth_token: str, tenant_id: str, insecure: bool) -> dict:
        client = FlightSQLClient(flight_url, token=auth_token, tenant_id=tenant_id, insecure=insecure)
        return client.server_info()

    def test_server_name(self, flight_url: str, auth_token: str, tenant_id: str, insecure: bool) -> None:
        info = self._get_info(flight_url, auth_token, tenant_id, insecure)
        assert info.get("vendor_name") == "Data Connect Hub"

    def test_server_version(self, flight_url: str, auth_token: str, tenant_id: str, insecure: bool) -> None:
        info = self._get_info(flight_url, auth_token, tenant_id, insecure)
        version = info.get("vendor_version")
        assert version is not None and len(version) > 0

    def test_arrow_version(self, flight_url: str, auth_token: str, tenant_id: str, insecure: bool) -> None:
        info = self._get_info(flight_url, auth_token, tenant_id, insecure)
        assert info.get("vendor_arrow_version") == "1.3"
