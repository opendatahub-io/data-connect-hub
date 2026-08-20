"""Tests for header construction utilities."""

from __future__ import annotations

import pytest

from data_connect_hub._auth import (
    TokenCache,
    _normalize_token,
    build_headers,
)
from data_connect_hub.exceptions import DCHConfigError


class TestNormalizeToken:
    def test_adds_bearer_prefix(self) -> None:
        assert _normalize_token("abc123") == "Bearer abc123"

    def test_preserves_existing_prefix(self) -> None:
        assert _normalize_token("Bearer abc123") == "Bearer abc123"

    def test_strips_double_prefix(self) -> None:
        assert _normalize_token("Bearer Bearer abc123") == "Bearer abc123"

    def test_case_insensitive_prefix(self) -> None:
        assert _normalize_token("bearer abc123") == "Bearer abc123"

    def test_case_insensitive_mixed_case(self) -> None:
        assert _normalize_token("BEARER abc123") == "Bearer abc123"

    def test_empty_token(self) -> None:
        assert _normalize_token("") == ""

    def test_rejects_basic_scheme(self) -> None:
        with pytest.raises(DCHConfigError, match="Unsupported auth scheme"):
            _normalize_token("Basic dXNlcjpwYXNz")

    def test_rejects_digest_scheme(self) -> None:
        with pytest.raises(DCHConfigError, match="Unsupported auth scheme"):
            _normalize_token("Digest realm=test")


class TestBuildHeaders:
    def test_all_headers(self) -> None:
        headers = build_headers(
            token="abc123",
            tenant_id="tenant-1",
        )
        assert headers == {
            "Authorization": "Bearer abc123",
            "x-tenant-id": "tenant-1",
        }

    def test_empty_values_excluded(self) -> None:
        headers = build_headers(token="", tenant_id="")
        assert headers == {}

    def test_bearer_prefixed_token_not_doubled(self) -> None:
        headers = build_headers(token="Bearer mytoken", tenant_id="t1")
        assert headers["Authorization"] == "Bearer mytoken"


class TestTokenCache:
    def test_calls_provider_once(self) -> None:
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        cache = TokenCache(provider)
        assert cache.get() == "token-1"
        assert cache.get() == "token-1"
        assert call_count == 1

    def test_refresh_calls_provider_again(self) -> None:
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            return f"token-{call_count}"

        cache = TokenCache(provider)
        assert cache.get() == "token-1"
        assert cache.refresh() == "token-2"
        assert cache.get() == "token-2"
        assert call_count == 2

    def test_provider_error_wrapped(self) -> None:
        def bad_provider() -> str:
            raise RuntimeError("login failed")

        cache = TokenCache(bad_provider)
        with pytest.raises(DCHConfigError, match=r"Token provider failed.*login failed"):
            cache.get()

    def test_provider_error_on_refresh_preserves_old_token(self) -> None:
        call_count = 0

        def provider() -> str:
            nonlocal call_count
            call_count += 1
            if call_count == 2:
                raise RuntimeError("refresh failed")
            return f"token-{call_count}"

        cache = TokenCache(provider)
        assert cache.get() == "token-1"
        with pytest.raises(DCHConfigError, match="Token provider failed"):
            cache.refresh()
        assert cache.get() == "token-1"
