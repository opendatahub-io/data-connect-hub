"""Report whether the SDK's TLS runs in the host's FIPS-validated OpenSSL.

The SDK implements no cryptography itself. It has two TLS paths:

* REST goes through ``httpx`` and Python's ``ssl`` module, which on RHEL-based
  images links the system OpenSSL.
* Flight goes through ``pyarrow.flight`` (gRPC). Only a pyarrow build that
  links the system OpenSSL, such as the one on the Red Hat AI Python index,
  uses the host's FIPS provider. PyPI wheels compile their own OpenSSL into
  pyarrow and ignore it.

:func:`fips_status` inspects the running process and makes no network calls.
"""

from __future__ import annotations

import ctypes
import importlib.metadata
import importlib.util
import os
import sys
import warnings
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from .exceptions import DCHConfigError, DCHFipsWarning

RestTls = Literal["fips", "not-fips", "unknown"]

_FIPS_ENABLED_PATH = Path("/proc/sys/crypto/fips_enabled")
_LIBSSL = "libssl.so.3"
_LIBCRYPTO = "libcrypto.so.3"
# Any libssl export works; this one exists in every OpenSSL 3 release.
_PROBE_SYMBOL = "SSL_CTX_new"

REQUIRE_FIPS_ENV = "DCH_REQUIRE_FIPS"
_TRUE = frozenset({"1", "true", "yes", "on"})
_FALSE = frozenset({"", "0", "false", "no", "off"})


@dataclass(frozen=True)
class FipsStatus:
    """Result of :func:`fips_status`.

    A FIPS-compliant setup reports ``host_fips=True``, ``rest_tls="fips"`` and,
    when Flight is used, ``flight_system_openssl=True``.

    Attributes
    ----------
    host_fips : bool
        The kernel runs in FIPS mode (``/proc/sys/crypto/fips_enabled`` is
        ``1``). ``False`` on other operating systems.
    rest_tls : str
        TLS used for REST calls (Python's ``ssl``):

        * ``"fips"``: Python uses the system OpenSSL 3 and its default
          properties are restricted to the FIPS provider.
        * ``"not-fips"``: the FIPS provider isn't enforced, or Python carries
          its own OpenSSL.
        * ``"unknown"``: not determinable, e.g. on macOS or Windows.
    flight_system_openssl : bool or None
        ``pyarrow.flight`` uses the system OpenSSL (e.g. Red Hat pyarrow), so
        Flight TLS follows the host's OpenSSL configuration, including FIPS.
        ``False`` if pyarrow carries its own OpenSSL (e.g. PyPI pyarrow), which
        never uses the host's FIPS provider. ``None`` if pyarrow isn't
        installed or fails to load, or on operating systems other than Linux.
    pyarrow_version : str or None
        Installed pyarrow version, or ``None`` if pyarrow isn't installed.
    pyarrow_error : str or None
        Why ``pyarrow.flight`` failed to import although pyarrow is installed,
        e.g. a missing system library such as ``libthrift`` when Red Hat
        pyarrow runs outside an RHOAI or AIPCC image.
    """

    host_fips: bool
    rest_tls: RestTls
    flight_system_openssl: bool | None
    pyarrow_version: str | None
    pyarrow_error: str | None


def fips_status() -> FipsStatus:
    """Report whether REST and Flight TLS use the host's FIPS-validated OpenSSL.

    Imports ``pyarrow.flight`` when pyarrow is installed. Makes no network calls.
    """
    pyarrow_version: str | None = None
    pyarrow_error: str | None = None
    flight_system_openssl: bool | None = None
    if _pyarrow_installed():
        pyarrow_version = _pyarrow_version()
        pyarrow_error = _pyarrow_import_error()
        if pyarrow_error is None:
            flight_system_openssl = _flight_system_openssl()
    return FipsStatus(
        host_fips=_host_fips(),
        rest_tls=_rest_tls(),
        flight_system_openssl=flight_system_openssl,
        pyarrow_version=pyarrow_version,
        pyarrow_error=pyarrow_error,
    )


def _host_fips() -> bool:
    try:
        return _FIPS_ENABLED_PATH.read_text().strip() == "1"
    except OSError:
        return False


def _rest_tls() -> RestTls:
    if not sys.platform.startswith("linux"):
        return "unknown"
    try:
        import _ssl
    except ImportError:
        return "unknown"
    path = getattr(_ssl, "__file__", None)
    if not path:  # built into the interpreter binary
        return "unknown"
    uses_system = _uses_system_openssl(path)
    if uses_system is None:
        return "unknown"
    if not uses_system:
        return "not-fips"
    fips = _openssl_fips_enabled()
    if fips is None:
        return "unknown"
    return "fips" if fips else "not-fips"


def _flight_system_openssl() -> bool | None:
    """Call only after ``pyarrow.flight`` imported successfully."""
    if not sys.platform.startswith("linux"):
        return None
    import pyarrow._flight

    path = getattr(pyarrow._flight, "__file__", None)
    return _uses_system_openssl(path) if path else None


def _uses_system_openssl(path: str) -> bool | None:
    """Whether the loaded library at *path* resolves libssl to the system copy.

    Looking a symbol up through a library handle also searches that library's
    dependencies. A library that links the system ``libssl.so.3`` resolves
    ``SSL_CTX_new`` to the same address as the system library; one that
    compiles OpenSSL in hides those symbols or resolves to its own copy.
    """
    try:
        system = ctypes.CDLL(_LIBSSL)
    except OSError:
        return False  # no system OpenSSL 3 to use
    try:
        lib = ctypes.CDLL(path)  # already loaded: returns the existing handle
    except OSError:
        return None
    address = _symbol_address(lib, _PROBE_SYMBOL)
    return address is not None and address == _symbol_address(system, _PROBE_SYMBOL)


def _symbol_address(lib: ctypes.CDLL, name: str) -> int | None:
    try:
        fn = getattr(lib, name)
    except AttributeError:
        return None
    address: int | None = ctypes.cast(fn, ctypes.c_void_p).value
    return address


def _openssl_fips_enabled() -> bool | None:
    """Whether the system OpenSSL 3 restricts lookups to the FIPS provider."""
    try:
        lib = ctypes.CDLL(_LIBCRYPTO)
        fn = lib.EVP_default_properties_is_fips_enabled
    except (OSError, AttributeError):
        return None
    fn.argtypes = [ctypes.c_void_p]
    fn.restype = ctypes.c_int
    return bool(fn(None) == 1)


def _pyarrow_installed() -> bool:
    """Check for pyarrow without importing it."""
    try:
        return importlib.util.find_spec("pyarrow") is not None
    except (ImportError, ValueError):
        return False


def _pyarrow_import_error() -> str | None:
    try:
        import pyarrow.flight  # noqa: F401
    except Exception as exc:  # any failure to load means Flight is unusable
        return f"{type(exc).__name__}: {exc}"
    return None


def _pyarrow_version() -> str | None:
    try:
        return importlib.metadata.version("pyarrow")
    except importlib.metadata.PackageNotFoundError:
        module = sys.modules.get("pyarrow")
        version = getattr(module, "__version__", None)
        return version if isinstance(version, str) else None


# -- require_fips enforcement ------------------------------------------------------------


def resolve_require_fips(value: bool | None) -> bool:
    """Return *value*, or the ``DCH_REQUIRE_FIPS`` environment variable when it's ``None``."""
    if value is not None:
        return value
    raw = os.environ.get(REQUIRE_FIPS_ENV, "").strip().lower()
    if raw in _TRUE:
        return True
    if raw in _FALSE:
        return False
    raise DCHConfigError(f"{REQUIRE_FIPS_ENV} must be one of 1/true/yes/on or 0/false/no/off, got {raw!r}")


def enforce_client_fips(*, insecure: bool) -> None:
    """Check the host and REST TLS for a ``require_fips`` client.

    Doesn't import pyarrow, so REST-only installs are unaffected; Flight is
    checked on first use by :func:`enforce_flight_fips`.
    """
    problems = []
    if not _host_fips():
        problems.append(f"the host is not in FIPS mode ({_FIPS_ENABLED_PATH} is not 1)")
    rest_tls = _rest_tls()
    if rest_tls != "fips":
        problems.append(
            f"REST TLS is {rest_tls!r}: Python's ssl module must use the system OpenSSL 3 "
            "with the FIPS provider enforced"
        )
    if problems:
        raise DCHConfigError(
            "require_fips is set but the environment is not FIPS-compliant: "
            + "; ".join(problems)
            + ". Run in a FIPS-enabled RHEL-based environment such as an RHOAI workbench on a FIPS cluster."
        )
    if insecure:
        warnings.warn(
            "require_fips is set with insecure=True: TLS still uses the FIPS provider, but server "
            "certificates are not verified. Pass ca_cert instead.",
            DCHFipsWarning,
            stacklevel=3,
        )


def enforce_flight_fips() -> None:
    """Check that Flight uses the system OpenSSL; call after ``pyarrow.flight`` imported."""
    system_openssl = _flight_system_openssl()
    if system_openssl is True:
        return
    version = _pyarrow_version() or "unknown"
    if system_openssl is False:
        reason = f"pyarrow {version} carries its own OpenSSL, which never uses the host's FIPS provider"
    else:
        reason = f"could not verify that pyarrow {version} uses the system OpenSSL"
    raise DCHConfigError(
        f"require_fips is set but {reason}. Install pyarrow from the Red Hat AI Python index "
        "(the default index in RHOAI workbench images) instead of PyPI."
    )
