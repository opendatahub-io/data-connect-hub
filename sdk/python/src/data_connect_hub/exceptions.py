"""SDK exception hierarchy mirroring DCH server errors."""

from __future__ import annotations

import httpx


class DCHError(Exception):
    """Base exception for all Data Connect Hub SDK errors."""


class DCHConnectionError(DCHError):
    """Failed to connect to the DCH server."""


class DCHTimeoutError(DCHError):
    """Request timed out."""


class DCHHTTPError(DCHError):
    """HTTP response indicated an error."""

    def __init__(self, message: str, status_code: int, body: str = "") -> None:
        super().__init__(message)
        self.status_code = status_code
        self.body = body


class DCHNotFoundError(DCHHTTPError):
    """404 Not Found."""


class DCHValidationError(DCHHTTPError):
    """400/422 Invalid request."""


class DCHAuthenticationError(DCHHTTPError):
    """401 Unauthorized."""


class DCHForbiddenError(DCHHTTPError):
    """403 Forbidden."""


class DCHServerError(DCHHTTPError):
    """5xx Server error."""


class DCHQueryError(DCHError):
    """SQL or query execution error."""


class DCHNoDataError(DCHError):
    """No data returned."""


class DCHConfigError(DCHError):
    """SDK configuration error."""


def map_http_error(response: httpx.Response) -> DCHHTTPError:
    """Convert an httpx error response to a typed exception."""
    body = response.text
    msg = f"HTTP {response.status_code}: {body}"

    match response.status_code:
        case 400 | 422:
            return DCHValidationError(msg, response.status_code, body)
        case 401:
            return DCHAuthenticationError(msg, response.status_code, body)
        case 403:
            return DCHForbiddenError(msg, response.status_code, body)
        case 404:
            return DCHNotFoundError(msg, response.status_code, body)
        case status if status >= 500:
            return DCHServerError(msg, response.status_code, body)
        case _:
            return DCHHTTPError(msg, response.status_code, body)
