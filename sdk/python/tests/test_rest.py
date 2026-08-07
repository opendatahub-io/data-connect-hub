"""Tests for the REST client."""

from __future__ import annotations

import json
from typing import Any

import httpx
import pytest

from data_connect_hub.exceptions import (
    DCHAuthenticationError,
    DCHConnectionError,
    DCHForbiddenError,
    DCHNotFoundError,
    DCHServerError,
    DCHTimeoutError,
    DCHValidationError,
)
from data_connect_hub.models import (
    CreateConnectionRequest,
    CreateConnectionTypeRequest,
    DataLocation,
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)
from data_connect_hub.rest import RestClient

from .conftest import SAMPLE_CONNECTION_JSON, SAMPLE_CONNECTION_TYPE_JSON


def _make_transport(
    status: int = 200,
    body: Any = None,
    *,
    assert_method: str | None = None,
    assert_path: str | None = None,
) -> httpx.MockTransport:
    def handler(request: httpx.Request) -> httpx.Response:
        if assert_method:
            assert request.method == assert_method
        if assert_path:
            assert request.url.path == assert_path
        return httpx.Response(
            status,
            json=body if body is not None else {},
            headers={"content-type": "application/json"},
        )

    return httpx.MockTransport(handler)


def _make_client(
    transport: httpx.MockTransport,
    api_base: str = "/api/v1/data",
    *,
    max_retries: int = 3,
    backoff_base: float = 0.0,
    backoff_max: float = 0.0,
) -> RestClient:
    http_client = httpx.Client(transport=transport, base_url="http://test")
    return RestClient(
        base_url="http://test",
        token="test-token",
        tenant_id="test-tenant",
        api_base=api_base,
        max_retries=max_retries,
        backoff_base=backoff_base,
        backoff_max=backoff_max,
        http_client=http_client,
    )


class TestListConnections:
    def test_returns_connections(self) -> None:
        transport = _make_transport(
            body=[SAMPLE_CONNECTION_JSON],
            assert_method="GET",
            assert_path="/api/v1/data/connections",
        )
        client = _make_client(transport)
        result = client.list_connections()
        assert len(result) == 1
        assert result[0].id == "123"
        assert result[0].namespace == "test-ns"

    def test_empty_list(self) -> None:
        transport = _make_transport(body=[])
        client = _make_client(transport)
        result = client.list_connections()
        assert result == []


class TestGetConnection:
    def test_returns_connection(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_JSON,
            assert_method="GET",
            assert_path="/api/v1/data/connections/123",
        )
        client = _make_client(transport)
        result = client.get_connection("123")
        assert result.id == "123"
        assert result.provider == "postgres"


class TestCreateConnection:
    def test_sends_post(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "POST"
            assert request.url.path == "/api/v1/data/connections"
            body = json.loads(request.content)
            assert body["name"] == "new-conn"
            assert body["provider"] == "postgres"
            return httpx.Response(201, json=SAMPLE_CONNECTION_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        req = CreateConnectionRequest(
            namespace="ns",
            name="new-conn",
            provider="postgres",
            format="arrow",
            location=DataLocation(url="pg://localhost"),
        )
        result = client.create_connection(req)
        assert result.id == "123"


class TestUpdateConnection:
    def test_sends_patch(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "PATCH"
            assert request.url.path == "/api/v1/data/connections/123"
            body = json.loads(request.content)
            assert body == {"name": "updated"}
            return httpx.Response(200, json=SAMPLE_CONNECTION_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        req = UpdateConnectionRequest(name="updated")
        client.update_connection("123", req)


class TestDeleteConnection:
    def test_sends_delete(self) -> None:
        transport = _make_transport(
            assert_method="DELETE",
            assert_path="/api/v1/data/connections/123",
        )
        client = _make_client(transport)
        client.delete_connection("123")


class TestConnectionTypes:
    def test_list(self) -> None:
        transport = _make_transport(
            body=[SAMPLE_CONNECTION_TYPE_JSON],
            assert_method="GET",
            assert_path="/api/v1/data/connection_types",
        )
        client = _make_client(transport)
        result = client.list_connection_types()
        assert len(result) == 1
        assert result[0].name == "postgres"

    def test_get(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_TYPE_JSON,
            assert_path="/api/v1/data/connection_types/ct-1",
        )
        client = _make_client(transport)
        result = client.get_connection_type("ct-1")
        assert result.id == "ct-1"

    def test_create(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "POST"
            assert request.url.path == "/api/v1/data/connection_types"
            body = json.loads(request.content)
            assert body["name"] == "mysql"
            assert body["description"] == "MySQL connector"
            return httpx.Response(201, json=SAMPLE_CONNECTION_TYPE_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        req = CreateConnectionTypeRequest(name="mysql", description="MySQL connector")
        result = client.create_connection_type(req)
        assert result.id == "ct-1"

    def test_update(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "PATCH"
            assert request.url.path == "/api/v1/data/connection_types/ct-1"
            body = json.loads(request.content)
            assert body == {"name": "renamed"}
            return httpx.Response(200, json=SAMPLE_CONNECTION_TYPE_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        req = UpdateConnectionTypeRequest(name="renamed")
        client.update_connection_type("ct-1", req)

    def test_delete(self) -> None:
        transport = _make_transport(
            assert_method="DELETE",
            assert_path="/api/v1/data/connection_types/ct-1",
        )
        client = _make_client(transport)
        client.delete_connection_type("ct-1")


class TestReadBytes:
    def test_returns_bytes(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.url.path == "/api/v1/data/ingestion/conn-1"
            return httpx.Response(200, content=b"raw-data")

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        result = client.read_bytes("conn-1")
        assert result == b"raw-data"


class TestHeaders:
    def test_auth_and_tenant_headers(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.headers["authorization"] == "Bearer test-token"
            assert request.headers["x-tenant-id"] == "test-tenant"
            return httpx.Response(200, json=[])

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        client.list_connections()


class TestCustomApiBase:
    def test_uses_custom_path(self) -> None:
        transport = _make_transport(
            body=[],
            assert_path="/v1/data/connections",
        )
        client = _make_client(transport, api_base="/v1/data")
        client.list_connections()


class TestErrorMapping:
    def test_400_raises_validation(self) -> None:
        transport = _make_transport(status=400, body={"error": "bad request"})
        client = _make_client(transport)
        with pytest.raises(DCHValidationError) as exc_info:
            client.list_connections()
        assert exc_info.value.status_code == 400

    def test_401_raises_authentication(self) -> None:
        transport = _make_transport(status=401, body={"error": "unauthorized"})
        client = _make_client(transport)
        with pytest.raises(DCHAuthenticationError):
            client.list_connections()

    def test_403_raises_forbidden(self) -> None:
        transport = _make_transport(status=403, body={"error": "forbidden"})
        client = _make_client(transport)
        with pytest.raises(DCHForbiddenError):
            client.list_connections()

    def test_404_raises_not_found(self) -> None:
        transport = _make_transport(status=404, body={"error": "not found"})
        client = _make_client(transport)
        with pytest.raises(DCHNotFoundError):
            client.get_connection("missing")

    def test_500_raises_server_error(self) -> None:
        transport = _make_transport(status=500, body={"error": "internal"})
        client = _make_client(transport)
        with pytest.raises(DCHServerError):
            client.list_connections()

    def test_long_body_truncated_in_message(self) -> None:
        long_body = {"error": "x" * 300}

        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(500, json=long_body, headers={"content-type": "application/json"})

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        with pytest.raises(DCHServerError) as exc_info:
            client.list_connections()
        assert len(str(exc_info.value)) <= 220
        assert len(exc_info.value.body) > 200


class TestTransportErrors:
    def test_connect_error_raises_dch_connection_error(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            raise httpx.ConnectError("connection refused")

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        with pytest.raises(DCHConnectionError, match="connection refused"):
            client.list_connections()

    def test_timeout_raises_dch_timeout_error(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            raise httpx.ReadTimeout("read timed out")

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        with pytest.raises(DCHTimeoutError, match="timed out"):
            client.list_connections()


class TestJsonParseSafety:
    def test_non_json_response_raises_dch_error(self) -> None:
        from data_connect_hub.exceptions import DCHError

        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(200, content=b"not json", headers={"content-type": "text/plain"})

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        with pytest.raises(DCHError, match="Unexpected non-JSON response"):
            client.list_connections()


class TestListUnwrapping:
    def test_handles_wrapped_items_response(self) -> None:
        transport = _make_transport(
            body={"items": [SAMPLE_CONNECTION_JSON]},
            assert_method="GET",
        )
        client = _make_client(transport)
        result = client.list_connections()
        assert len(result) == 1
        assert result[0].id == "123"

    def test_handles_bare_array_response(self) -> None:
        transport = _make_transport(
            body=[SAMPLE_CONNECTION_JSON],
            assert_method="GET",
        )
        client = _make_client(transport)
        result = client.list_connections()
        assert len(result) == 1


class TestRetry:
    def test_retries_on_503_then_succeeds(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            if call_count < 3:
                return httpx.Response(503, json={"error": "unavailable"})
            return httpx.Response(200, json=[SAMPLE_CONNECTION_JSON])

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=3, backoff_base=0.0)
        result = client.list_connections()
        assert len(result) == 1
        assert call_count == 3

    def test_retries_on_429_respects_retry_after(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            if call_count == 1:
                return httpx.Response(429, json={}, headers={"retry-after": "0"})
            return httpx.Response(200, json=[])

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=2, backoff_base=0.0)
        result = client.list_connections()
        assert result == []
        assert call_count == 2

    def test_exhausts_retries_raises_last_error(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(503, json={"error": "down"})

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=2, backoff_base=0.0)
        from data_connect_hub.exceptions import DCHServerError

        with pytest.raises(DCHServerError):
            client.list_connections()

    def test_no_retry_on_non_idempotent_by_default(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            return httpx.Response(503, json={"error": "unavailable"})

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=3, backoff_base=0.0)
        from data_connect_hub.exceptions import DCHServerError
        from data_connect_hub.models import CreateConnectionRequest, DataLocation

        req = CreateConnectionRequest(
            namespace="ns",
            name="c",
            provider="pg",
            format="arrow",
            location=DataLocation(url="pg://localhost"),
        )
        with pytest.raises(DCHServerError):
            client.create_connection(req)
        assert call_count == 1

    def test_retries_on_connect_error(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            if call_count < 2:
                raise httpx.ConnectError("connection refused")
            return httpx.Response(200, json=[])

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=3, backoff_base=0.0)
        result = client.list_connections()
        assert result == []
        assert call_count == 2

    def test_retries_on_timeout(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            if call_count < 2:
                raise httpx.ReadTimeout("read timed out")
            return httpx.Response(200, json=[])

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=3, backoff_base=0.0)
        result = client.list_connections()
        assert result == []
        assert call_count == 2

    def test_no_retry_when_disabled(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            return httpx.Response(503, json={"error": "unavailable"})

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=0, backoff_base=0.0)
        from data_connect_hub.exceptions import DCHServerError

        with pytest.raises(DCHServerError):
            client.list_connections()
        assert call_count == 1
