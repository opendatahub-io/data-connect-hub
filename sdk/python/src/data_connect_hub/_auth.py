"""Header construction for REST authentication."""

from __future__ import annotations

from .exceptions import DCHConfigError

_BEARER_PREFIX = "Bearer "


def _normalize_token(token: str) -> str:
    """Ensure *token* carries the ``Bearer`` prefix exactly once.

    Raises ``DCHConfigError`` if the token appears to use a non-Bearer
    auth scheme (e.g. ``Basic``).
    """
    if not token:
        return token

    while token.startswith(_BEARER_PREFIX):
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
    """Build HTTP headers for REST API calls."""
    headers: dict[str, str] = {}
    if token:
        headers["Authorization"] = _normalize_token(token)
    if tenant_id:
        headers["x-tenant-id"] = tenant_id
    return headers
