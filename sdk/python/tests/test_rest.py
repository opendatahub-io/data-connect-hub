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
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)
from data_connect_hub.rest import RestClient

from .conftest import (
    SAMPLE_CONNECTION_JSON,
    SAMPLE_CONNECTION_TYPE_JSON,
    SAMPLE_CONNECTION_TYPE_WRAPPED_JSON,
    SAMPLE_CONNECTION_WRAPPED_JSON,
)


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
        assert result[0].data_connection_type_id == "postgres"

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
        assert result.data_connection_type_id == "postgres"


class TestCreateConnection:
    def test_sends_post(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "POST"
            assert request.url.path == "/api/v1/data/connections"
            body = json.loads(request.content)
            assert body["name"] == "new-conn"
            assert body["data_connection_type_id"] == "postgres"
            assert body["format"] == "tabular"
            return httpx.Response(201, json=SAMPLE_CONNECTION_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        req = CreateConnectionRequest(
            name="new-conn",
            data_connection_type_id="postgres",
            format="tabular",
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
            assert_path="/api/v1/data/connection-types",
        )
        client = _make_client(transport)
        result = client.list_connection_types()
        assert len(result) == 1
        assert result[0].name == "postgres"

    def test_get(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_TYPE_JSON,
            assert_path="/api/v1/data/connection-types/ct-1",
        )
        client = _make_client(transport)
        result = client.get_connection_type("ct-1")
        assert result.id == "ct-1"

    def test_create(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "POST"
            assert request.url.path == "/api/v1/data/connection-types"
            body = json.loads(request.content)
            assert body["name"] == "mysql"
            assert body["provider"] == "mysql"
            assert body["description"] == "MySQL connector"
            return httpx.Response(201, json=SAMPLE_CONNECTION_TYPE_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        req = CreateConnectionTypeRequest(name="mysql", provider="mysql", description="MySQL connector")
        result = client.create_connection_type(req)
        assert result.id == "ct-1"

    def test_update(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "PATCH"
            assert request.url.path == "/api/v1/data/connection-types/ct-1"
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
            assert_path="/api/v1/data/connection-types/ct-1",
        )
        client = _make_client(transport)
        client.delete_connection_type("ct-1")


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

    def test_handles_resource_list_with_total_count(self) -> None:
        transport = _make_transport(
            body={"total_count": 1, "items": [SAMPLE_CONNECTION_TYPE_WRAPPED_JSON]},
            assert_method="GET",
        )
        client = _make_client(transport)
        result = client.list_connection_types()
        assert len(result) == 1
        assert result[0].id == "ct-1"
        assert result[0].name == "postgres"

    def test_handles_bare_array_response(self) -> None:
        transport = _make_transport(
            body=[SAMPLE_CONNECTION_JSON],
            assert_method="GET",
        )
        client = _make_client(transport)
        result = client.list_connections()
        assert len(result) == 1


class TestWrappedResourceFormat:
    def test_get_connection_wrapped(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_WRAPPED_JSON,
            assert_method="GET",
            assert_path="/api/v1/data/connections/123",
        )
        client = _make_client(transport)
        result = client.get_connection("123")
        assert result.id == "123"
        assert result.data_connection_type_id == "postgres"
        from data_connect_hub.models import AdminSecretRef

        assert result.admin == AdminSecretRef(secret_ref="secret/test-conn")

    def test_get_connection_type_wrapped(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_TYPE_WRAPPED_JSON,
            assert_method="GET",
            assert_path="/api/v1/data/connection-types/ct-1",
        )
        client = _make_client(transport)
        result = client.get_connection_type("ct-1")
        assert result.id == "ct-1"
        assert result.name == "postgres"
        assert result.provider == "postgres"
        assert result.tenant_id == "default"

    def test_create_connection_type_returns_wrapped(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "POST"
            return httpx.Response(201, json=SAMPLE_CONNECTION_TYPE_WRAPPED_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        req = CreateConnectionTypeRequest(name="pg", provider="postgres")
        result = client.create_connection_type(req)
        assert result.id == "ct-1"
        assert result.name == "postgres"


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
        from data_connect_hub.models import CreateConnectionRequest

        req = CreateConnectionRequest(
            name="c",
            data_connection_type_id="pg",
            format="tabular",
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


class TestTokenProviderGuard:
    def test_token_and_provider_raises(self) -> None:
        from data_connect_hub.exceptions import DCHConfigError

        with pytest.raises(DCHConfigError, match="Cannot specify both"):
            RestClient(
                base_url="http://test",
                token="tok",
                tenant_id="t1",
                token_provider=lambda: "fresh",
            )


class TestTokenProvider:
    def test_provider_called_once_and_cached(self) -> None:
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        captured_headers: list[dict[str, str]] = []

        def handler(request: httpx.Request) -> httpx.Response:
            captured_headers.append(dict(request.headers))
            return httpx.Response(200, json=[])

        transport = httpx.MockTransport(handler)
        http_client = httpx.Client(transport=transport, base_url="http://test")
        client = RestClient(
            base_url="http://test",
            token="",
            tenant_id="t1",
            token_provider=provider,
            http_client=http_client,
        )

        client.list_connections()
        client.list_connections()

        assert call_count == 1
        assert captured_headers[0]["authorization"] == "Bearer token-1"
        assert captured_headers[1]["authorization"] == "Bearer token-1"

    def test_401_triggers_token_refresh_and_retry(self) -> None:
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        request_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal request_count
            request_count += 1
            if request_count == 1:
                return httpx.Response(401, json={"error": "unauthorized"})
            return httpx.Response(200, json=[])

        transport = httpx.MockTransport(handler)
        http_client = httpx.Client(transport=transport, base_url="http://test")
        client = RestClient(
            base_url="http://test",
            token="",
            tenant_id="t1",
            token_provider=provider,
            http_client=http_client,
            max_retries=0,
        )

        result = client.list_connections()

        assert call_count == 2
        assert request_count == 2
        assert result == []

    def test_401_after_refresh_raises(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(401, json={"error": "unauthorized"})

        transport = httpx.MockTransport(handler)
        http_client = httpx.Client(transport=transport, base_url="http://test")
        client = RestClient(
            base_url="http://test",
            token="",
            tenant_id="t1",
            token_provider=lambda: "bad-token",
            http_client=http_client,
            max_retries=0,
        )

        with pytest.raises(DCHAuthenticationError):
            client.list_connections()

    def test_401_without_provider_raises_immediately(self) -> None:
        transport = _make_transport(status=401, body={"error": "unauthorized"})
        client = _make_client(transport)
        with pytest.raises(DCHAuthenticationError):
            client.list_connections()
