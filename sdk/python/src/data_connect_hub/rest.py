"""Synchronous REST client for DCH connection management APIs."""

from __future__ import annotations

import json as _json
import logging
import random
import time
from typing import Any

import httpx

from ._auth import build_headers
from .exceptions import DCHConnectionError, DCHError, DCHTimeoutError, map_http_error
from .models import (
    ConnectionType,
    CreateConnectionRequest,
    CreateConnectionTypeRequest,
    DataConnection,
    UpdateConnectionRequest,
    UpdateConnectionTypeRequest,
)

_DEFAULT_API_BASE = "/api/v1/data"
_RETRYABLE_STATUS_CODES = frozenset({429, 502, 503, 504})
_IDEMPOTENT_METHODS = frozenset({"GET", "HEAD", "PUT", "DELETE", "OPTIONS"})

_log = logging.getLogger(__name__)


def _unwrap_list(data: Any) -> list[Any]:
    """Extract items from either a bare JSON array or a ``{"items": [...]}`` wrapper."""
    if isinstance(data, list):
        return data
    if isinstance(data, dict) and "items" in data:
        items: list[Any] = data["items"]
        return items
    raise DCHError(f'Unexpected response format: expected list or {{"items": [...]}}, got {type(data).__name__}')


class RestClient:
    """httpx-based REST client for DCH.

    Supports configurable retry with exponential backoff for transient errors
    (429, 502, 503, 504) and connectivity/timeout failures on idempotent methods.

    Parameters
    ----------
    max_retries : int
        Maximum number of retry attempts (default 3). Set to 0 to disable.
    backoff_base : float
        Base delay in seconds for exponential backoff (default 0.5).
        Actual delay = ``backoff_base * 2 ** attempt`` capped at ``backoff_max``.
    backoff_max : float
        Maximum backoff delay in seconds (default 30.0).
    retry_methods : frozenset[str] | None
        HTTP methods eligible for retry. Defaults to idempotent methods only.
        Pass ``None`` to retry all methods (use with caution for POST/PATCH).
    """

    def __init__(
        self,
        base_url: str,
        token: str,
        tenant_id: str,
        *,
        api_base: str = _DEFAULT_API_BASE,
        timeout: float = 30.0,
        max_retries: int = 3,
        backoff_base: float = 0.5,
        backoff_max: float = 30.0,
        retry_methods: frozenset[str] | None = _IDEMPOTENT_METHODS,
        http_client: httpx.Client | None = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._token = token
        self._tenant_id = tenant_id
        self._api_base = api_base
        self._max_retries = max_retries
        self._backoff_base = backoff_base
        self._backoff_max = backoff_max
        self._retry_methods = retry_methods
        self._owns_client = http_client is None
        self._client = http_client or httpx.Client(
            base_url=self._base_url,
            timeout=timeout,
        )

    def close(self) -> None:
        if self._owns_client:
            self._client.close()

    def _headers(self) -> dict[str, str]:
        return build_headers(
            token=self._token,
            tenant_id=self._tenant_id,
        )

    def _is_retryable(self, method: str) -> bool:
        if self._retry_methods is None:
            return True
        return method.upper() in self._retry_methods

    def _backoff_delay(self, attempt: int) -> float:
        delay: float = min(self._backoff_base * (2**attempt), self._backoff_max)
        return delay * random.uniform(0.5, 1.0)

    def _request(
        self,
        method: str,
        path: str,
        *,
        json: dict[str, object] | None = None,
    ) -> httpx.Response:
        last_exc: DCHError | None = None
        retryable = self._is_retryable(method)
        attempts = self._max_retries + 1 if retryable else 1

        for attempt in range(attempts):
            try:
                resp = self._client.request(
                    method,
                    f"{self._api_base}{path}",
                    headers=self._headers(),
                    json=json,
                )
            except httpx.ConnectError as exc:
                last_exc = DCHConnectionError(f"Failed to connect to {self._base_url}: {exc}")
                if attempt < attempts - 1:
                    delay = self._backoff_delay(attempt)
                    _log.debug("Retry %d/%d after ConnectError, sleeping %.2fs", attempt + 1, self._max_retries, delay)
                    time.sleep(delay)
                    continue
                raise last_exc from exc
            except httpx.TimeoutException as exc:
                last_exc = DCHTimeoutError(f"Request timed out: {exc}")
                if attempt < attempts - 1:
                    delay = self._backoff_delay(attempt)
                    _log.debug("Retry %d/%d after timeout, sleeping %.2fs", attempt + 1, self._max_retries, delay)
                    time.sleep(delay)
                    continue
                raise last_exc from exc

            if resp.status_code in _RETRYABLE_STATUS_CODES and attempt < attempts - 1:
                delay = self._retry_after(resp, attempt)
                _log.debug(
                    "Retry %d/%d after HTTP %d, sleeping %.2fs",
                    attempt + 1,
                    self._max_retries,
                    resp.status_code,
                    delay,
                )
                time.sleep(delay)
                continue

            if resp.status_code >= 400:
                raise map_http_error(resp)
            return resp

        # Unreachable: every loop iteration returns, raises, or continues.
        # Kept as a safety net for future refactors.
        raise last_exc or DCHError("Request failed after retries")  # pragma: no cover

    def _retry_after(self, resp: httpx.Response, attempt: int) -> float:
        """Parse numeric Retry-After header or fall back to exponential backoff.

        HTTP-date format (RFC 7231) is not supported; the fallback handles it.
        """
        retry_after = resp.headers.get("retry-after")
        if retry_after:
            try:
                return min(float(retry_after), self._backoff_max)
            except ValueError:
                pass
        return self._backoff_delay(attempt)

    def _parse_json(self, resp: httpx.Response) -> Any:
        """Safely parse response JSON, raising DCHError on decode failure."""
        try:
            return resp.json()
        except _json.JSONDecodeError as exc:
            raise DCHError(f"Unexpected non-JSON response (status {resp.status_code}): {resp.text[:200]}") from exc

    # -- Connections CRUD --

    def list_connections(self) -> list[DataConnection]:
        resp = self._request("GET", "/connections")
        return [DataConnection.model_validate(c) for c in _unwrap_list(self._parse_json(resp))]

    def get_connection(self, connection_id: str) -> DataConnection:
        resp = self._request("GET", f"/connections/{connection_id}")
        return DataConnection.model_validate(self._parse_json(resp))

    def create_connection(self, request: CreateConnectionRequest) -> DataConnection:
        resp = self._request(
            "POST",
            "/connections",
            json=request.model_dump(exclude_none=True),
        )
        return DataConnection.model_validate(self._parse_json(resp))

    def update_connection(self, connection_id: str, request: UpdateConnectionRequest) -> DataConnection:
        resp = self._request(
            "PATCH",
            f"/connections/{connection_id}",
            json=request.model_dump(exclude_none=True),
        )
        return DataConnection.model_validate(self._parse_json(resp))

    def delete_connection(self, connection_id: str) -> None:
        self._request("DELETE", f"/connections/{connection_id}")

    # -- Connection Types CRUD --

    def list_connection_types(self) -> list[ConnectionType]:
        resp = self._request("GET", "/connection_types")
        return [ConnectionType.model_validate(ct) for ct in _unwrap_list(self._parse_json(resp))]

    def get_connection_type(self, type_id: str) -> ConnectionType:
        resp = self._request("GET", f"/connection_types/{type_id}")
        return ConnectionType.model_validate(self._parse_json(resp))

    def create_connection_type(self, request: CreateConnectionTypeRequest) -> ConnectionType:
        resp = self._request(
            "POST",
            "/connection_types",
            json=request.model_dump(exclude_none=True),
        )
        return ConnectionType.model_validate(self._parse_json(resp))

    def update_connection_type(self, type_id: str, request: UpdateConnectionTypeRequest) -> ConnectionType:
        resp = self._request(
            "PATCH",
            f"/connection_types/{type_id}",
            json=request.model_dump(exclude_none=True),
        )
        return ConnectionType.model_validate(self._parse_json(resp))

    def delete_connection_type(self, type_id: str) -> None:
        self._request("DELETE", f"/connection_types/{type_id}")

    # -- Unstructured ingestion --

    def ingest(self, connection_id: str) -> bytes:
        """Fetch raw unstructured data for a connection."""
        resp = self._request("GET", f"/ingestion/{connection_id}")
        return resp.content
