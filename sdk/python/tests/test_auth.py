"""Tests for header construction utilities."""

from __future__ import annotations

import pytest

from data_connect_hub._auth import (
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
