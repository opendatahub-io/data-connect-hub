"""Header construction and token caching for REST and Flight SQL authentication."""

from __future__ import annotations

import threading
from collections.abc import Callable

from .exceptions import DCHConfigError

_BEARER_PREFIX = "Bearer "


def _normalize_token(token: str) -> str:
    """Ensure *token* carries the ``Bearer`` prefix exactly once.

    Raises ``DCHConfigError`` if the token appears to use a non-Bearer
    auth scheme (e.g. ``Basic``).
    """
    if not token:
        return token

    while token.upper().startswith(_BEARER_PREFIX.upper()):
        token = token[len(_BEARER_PREFIX) :]

    detected_scheme = token.split(None, 1)[0] if token else ""
    if detected_scheme.lower() in ("basic", "digest"):
        raise DCHConfigError(f"Unsupported auth scheme {detected_scheme!r} — pass only the raw Bearer token value")

    return f"{_BEARER_PREFIX}{token}"


def build_headers(
    *,
    token: str,
    tenant_id: str,
) -> dict[str, str]:
    """Build standard HTTP headers for authentication."""
    headers: dict[str, str] = {}
    if token:
        headers["Authorization"] = _normalize_token(token)
    if tenant_id:
        headers["x-tenant-id"] = tenant_id
    return headers


class TokenCache:
    """Thread-safe cache that calls a provider once and reuses the token until refreshed."""

    def __init__(self, provider: Callable[[], str]) -> None:
        self._provider = provider
        self._token: str | None = None
        self._lock = threading.Lock()

    def _call_provider(self) -> str:
        try:
            return self._provider()
        except Exception as exc:
            raise DCHConfigError(f"Token provider failed: {exc}") from exc

    def get(self) -> str:
        with self._lock:
            if self._token is None:
                self._token = self._call_provider()
            return self._token

    def refresh(self) -> str:
        with self._lock:
            self._token = self._call_provider()
            return self._token


ADBC_HEADER_PREFIX = "adbc.flight.sql.rpc.call_header."
