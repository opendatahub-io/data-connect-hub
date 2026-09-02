"""Tests for the REST client."""

from __future__ import annotations

import json
from typing import Any

import httpx
import pytest

from data_connect_hub._rest import _DEFAULT_API_BASE, RestClient
from data_connect_hub.exceptions import (
    DCHAuthenticationError,
    DCHConfigError,
    DCHConnectionError,
    DCHForbiddenError,
    DCHNotFoundError,
    DCHResponseError,
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

from .conftest import (
    SAMPLE_CONNECTION_JSON,
    SAMPLE_CONNECTION_TYPE_JSON,
    SAMPLE_CONNECTION_TYPE_WRAPPED_JSON,
    SAMPLE_CONNECTION_WRAPPED_JSON,
)


def _api_path(path: str) -> str:
    """Construct a full API path from a relative endpoint path."""
    return f"{_DEFAULT_API_BASE}{path}"


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
    api_base: str = _DEFAULT_API_BASE,
    *,
    max_retries: int = 3,
    backoff_base: float = 0.0,
    backoff_max: float = 0.0,
) -> RestClient:
    http_client = httpx.Client(transport=transport, base_url="http://test")
    return RestClient(
        url="http://test",
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
            assert_path=_api_path("/connections"),
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
            assert_path=_api_path("/connections/123"),
        )
        client = _make_client(transport)
        result = client.get_connection("123")
        assert result.id == "123"
        assert result.data_connection_type_id == "postgres"


class TestCreateConnection:
    def test_sends_post(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "POST"
            assert request.url.path == _api_path("/connections")
            body = json.loads(request.content)
            assert body["name"] == "new-conn"
            assert body["data_connection_type_id"] == "postgres"
            assert body["format"] == "tabular"
            return httpx.Response(201, json=SAMPLE_CONNECTION_JSON)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        from data_connect_hub.models import CredentialsRef

        req = CreateConnectionRequest(
            name="new-conn",
            data_connection_type_id="postgres",
            format="tabular",
            credentials_ref=CredentialsRef(secret="secret/test-conn"),
        )
        result = client.create_connection(req)
        assert result.id == "123"


class TestUpdateConnection:
    def test_sends_patch(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "PATCH"
            assert request.url.path == _api_path("/connections/123")
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
            assert_path=_api_path("/connections/123"),
        )
        client = _make_client(transport)
        client.delete_connection("123")


class TestConnectionTypes:
    def test_list(self) -> None:
        transport = _make_transport(
            body=[SAMPLE_CONNECTION_TYPE_JSON],
            assert_method="GET",
            assert_path=_api_path("/connection-types"),
        )
        client = _make_client(transport)
        result = client.list_connection_types()
        assert len(result) == 1
        assert result[0].name == "postgres"

    def test_get(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_TYPE_JSON,
            assert_path=_api_path("/connection-types/ct-1"),
        )
        client = _make_client(transport)
        result = client.get_connection_type("ct-1")
        assert result.id == "ct-1"

    def test_create(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            assert request.method == "POST"
            assert request.url.path == _api_path("/connection-types")
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
            assert request.url.path == _api_path("/connection-types/ct-1")
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
            assert_path=_api_path("/connection-types/ct-1"),
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

    def test_protocol_error_raises_dch_connection_error(self) -> None:
        """A mid-response disconnect is neither a ConnectError nor a timeout."""

        def handler(request: httpx.Request) -> httpx.Response:
            raise httpx.RemoteProtocolError("server disconnected", request=request)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=0)
        with pytest.raises(DCHConnectionError, match="RemoteProtocolError"):
            client.list_connections()

    def test_pool_timeout_raises_dch_timeout_error(self) -> None:
        def handler(request: httpx.Request) -> httpx.Response:
            raise httpx.PoolTimeout("pool exhausted")

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=0)
        with pytest.raises(DCHTimeoutError, match="timed out"):
            client.list_connections()

    def test_non_transport_request_error_is_not_retried(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            raise httpx.TooManyRedirects("redirect loop", request=request)

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=3)
        with pytest.raises(DCHConnectionError, match="TooManyRedirects"):
            client.list_connections()
        assert call_count == 1

    def test_retries_on_protocol_error(self) -> None:
        call_count = 0

        def handler(request: httpx.Request) -> httpx.Response:
            nonlocal call_count
            call_count += 1
            if call_count < 2:
                raise httpx.RemoteProtocolError("server disconnected", request=request)
            return httpx.Response(200, json=[])

        transport = httpx.MockTransport(handler)
        client = _make_client(transport, max_retries=3)
        assert client.list_connections() == []
        assert call_count == 2


class TestJsonParseSafety:
    def test_non_json_response_raises_dch_error(self) -> None:
        from data_connect_hub.exceptions import DCHError

        def handler(request: httpx.Request) -> httpx.Response:
            return httpx.Response(200, content=b"not json", headers={"content-type": "text/plain"})

        transport = httpx.MockTransport(handler)
        client = _make_client(transport)
        with pytest.raises(DCHError, match="Unexpected non-JSON response"):
            client.list_connections()

    def test_schema_mismatch_raises_dch_response_error(self) -> None:
        """A payload that fails validation must not leak pydantic.ValidationError."""
        transport = _make_transport(body={"id": "1"})
        client = _make_client(transport)
        with pytest.raises(DCHResponseError, match="unexpected DataConnection payload"):
            client.get_connection("1")

    def test_schema_mismatch_in_list_raises_dch_response_error(self) -> None:
        transport = _make_transport(body=[{"id": "1"}])
        client = _make_client(transport)
        with pytest.raises(DCHResponseError):
            client.list_connections()

    def test_unexpected_list_envelope_raises_dch_response_error(self) -> None:
        transport = _make_transport(body={"connections": []})
        client = _make_client(transport)
        with pytest.raises(DCHResponseError, match="Unexpected response format"):
            client.list_connections()


class TestPathSegmentEncoding:
    """Ids are percent-encoded so a hostile or malformed id cannot retarget the request."""

    @pytest.mark.parametrize(
        ("connection_id", "expected_path"),
        [
            ("../../../../evil", _api_path("/connections/..%2F..%2F..%2F..%2Fevil")),
            ("abc?force=true", _api_path("/connections/abc%3Fforce%3Dtrue")),
            ("a/b", _api_path("/connections/a%2Fb")),
            ("id with space", _api_path("/connections/id%20with%20space")),
            ("a#frag", _api_path("/connections/a%23frag")),
        ],
    )
    def test_connection_id_is_encoded(self, connection_id: str, expected_path: str) -> None:
        seen: list[str] = []

        def handler(request: httpx.Request) -> httpx.Response:
            seen.append(request.url.raw_path.decode())
            return httpx.Response(204)

        client = _make_client(httpx.MockTransport(handler))
        client.delete_connection(connection_id)
        assert seen == [expected_path]

    def test_type_id_is_encoded(self) -> None:
        seen: list[str] = []

        def handler(request: httpx.Request) -> httpx.Response:
            seen.append(request.url.raw_path.decode())
            return httpx.Response(204)

        client = _make_client(httpx.MockTransport(handler))
        client.delete_connection_type("../connections/123")
        assert seen == [_api_path("/connection-types/..%2Fconnections%2F123")]

    def test_ordinary_id_is_unchanged(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_JSON,
            assert_path=_api_path("/connections/550e8400-e29b-41d4-a716-446655440000"),
        )
        client = _make_client(transport)
        client.get_connection("550e8400-e29b-41d4-a716-446655440000")

    @pytest.mark.parametrize("bad_id", ["", "   "])
    def test_blank_id_rejected(self, bad_id: str) -> None:
        client = _make_client(_make_transport())
        with pytest.raises(DCHConfigError, match="connection_id must be a non-empty string"):
            client.get_connection(bad_id)


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
            assert_path=_api_path("/connections/123"),
        )
        client = _make_client(transport)
        result = client.get_connection("123")
        assert result.id == "123"
        assert result.data_connection_type_id == "postgres"
        from data_connect_hub.models import CredentialsRef

        assert result.credentials_ref == CredentialsRef(secret="secret/test-conn")

    def test_get_connection_type_wrapped(self) -> None:
        transport = _make_transport(
            body=SAMPLE_CONNECTION_TYPE_WRAPPED_JSON,
            assert_method="GET",
            assert_path=_api_path("/connection-types/ct-1"),
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
        from data_connect_hub.models import CreateConnectionRequest, CredentialsRef

        req = CreateConnectionRequest(
            name="c",
            data_connection_type_id="pg",
            format="tabular",
            credentials_ref=CredentialsRef(secret="secret/test"),
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
                url="http://test",
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
            url="http://test",
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
            url="http://test",
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
            url="http://test",
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


class TestCodeRabbitFixes:
    def test_unwrap_list_rejects_null_items(self) -> None:
        """Null items value raises DCHResponseError, not TypeError."""
        transport = _make_transport(body={"items": None})
        client = _make_client(transport)
        with pytest.raises(DCHResponseError, match="must be a list"):
            client.list_connections()

    def test_unwrap_list_rejects_string_items(self) -> None:
        transport = _make_transport(body={"items": "not a list"})
        client = _make_client(transport)
        with pytest.raises(DCHResponseError, match="must be a list"):
            client.list_connections()

    def test_validate_catches_typeerror_from_malformed_envelope(self) -> None:
        """Malformed resource envelope raises DCHResponseError, not TypeError."""
        transport = _make_transport(body={"metadata": None, "resource": {}})
        client = _make_client(transport)
        with pytest.raises(DCHResponseError, match=r"malformed.*payload"):
            client.get_connection("conn-id")
