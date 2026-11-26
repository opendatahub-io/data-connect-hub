"""Tests for fips_status()."""

from __future__ import annotations

import builtins
import sys
import types
from pathlib import Path
from typing import Any

import pytest

import data_connect_hub
from data_connect_hub import _fips
from data_connect_hub._fips import FipsStatus, fips_status

SYSTEM_SSL_CTX_NEW = 0x7F0000001000


def fake_cdll(monkeypatch: pytest.MonkeyPatch, libs: dict[str, dict[str, Any]]) -> list[str]:
    """Replace ctypes.CDLL with libraries exposing the given symbols; missing names fail to load."""
    loaded: list[str] = []

    def cdll(name: str) -> types.SimpleNamespace:
        if name not in libs:
            raise OSError(f"{name}: cannot open shared object file")
        loaded.append(name)
        return types.SimpleNamespace(**libs[name])

    monkeypatch.setattr(_fips.ctypes, "CDLL", cdll)
    return loaded


@pytest.fixture
def linux(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(_fips.sys, "platform", "linux")


class TestHostFips:
    @pytest.mark.parametrize(("content", "expected"), [("1\n", True), ("0\n", False), ("", False)])
    def test_reads_proc_file(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, content: str, expected: bool
    ) -> None:
        path = tmp_path / "fips_enabled"
        path.write_text(content)
        monkeypatch.setattr(_fips, "_FIPS_ENABLED_PATH", path)
        assert _fips._host_fips() is expected

    def test_missing_file_is_not_fips(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setattr(_fips, "_FIPS_ENABLED_PATH", tmp_path / "absent")
        assert _fips._host_fips() is False


@pytest.mark.usefixtures("linux")
class TestUsesSystemOpenssl:
    def test_resolves_to_system_libssl(self, monkeypatch: pytest.MonkeyPatch) -> None:
        # Red Hat pyarrow: the lookup through the library finds the system libssl.so.3.
        fake_cdll(
            monkeypatch,
            {"libssl.so.3": {"SSL_CTX_new": SYSTEM_SSL_CTX_NEW}, "/lib/flight.so": {"SSL_CTX_new": SYSTEM_SSL_CTX_NEW}},
        )
        assert _fips._uses_system_openssl("/lib/flight.so") is True

    def test_hidden_bundled_openssl(self, monkeypatch: pytest.MonkeyPatch) -> None:
        # PyPI pyarrow: OpenSSL is compiled in with hidden symbols.
        fake_cdll(monkeypatch, {"libssl.so.3": {"SSL_CTX_new": SYSTEM_SSL_CTX_NEW}, "/lib/flight.so": {}})
        assert _fips._uses_system_openssl("/lib/flight.so") is False

    def test_other_openssl_copy(self, monkeypatch: pytest.MonkeyPatch) -> None:
        # A vendored libssl-<hash>.so.3 or exported static copy resolves elsewhere.
        fake_cdll(
            monkeypatch,
            {"libssl.so.3": {"SSL_CTX_new": SYSTEM_SSL_CTX_NEW}, "/lib/flight.so": {"SSL_CTX_new": 0x7F0000009000}},
        )
        assert _fips._uses_system_openssl("/lib/flight.so") is False

    def test_no_system_openssl3(self, monkeypatch: pytest.MonkeyPatch) -> None:
        loaded = fake_cdll(monkeypatch, {"/lib/flight.so": {"SSL_CTX_new": SYSTEM_SSL_CTX_NEW}})
        assert _fips._uses_system_openssl("/lib/flight.so") is False
        assert loaded == []

    def test_library_not_loadable(self, monkeypatch: pytest.MonkeyPatch) -> None:
        fake_cdll(monkeypatch, {"libssl.so.3": {"SSL_CTX_new": SYSTEM_SSL_CTX_NEW}})
        assert _fips._uses_system_openssl("/lib/flight.so") is None


class TestOpensslFipsEnabled:
    class _Func:
        def __init__(self, result: int) -> None:
            self.result = result
            self.calls: list[tuple[Any, ...]] = []

        def __call__(self, *args: Any) -> int:
            self.calls.append(args)
            return self.result

    @pytest.mark.parametrize(("value", "expected"), [(1, True), (0, False)])
    def test_reads_default_properties(self, monkeypatch: pytest.MonkeyPatch, value: int, expected: bool) -> None:
        fn = self._Func(value)
        fake_cdll(monkeypatch, {"libcrypto.so.3": {"EVP_default_properties_is_fips_enabled": fn}})
        assert _fips._openssl_fips_enabled() is expected
        assert fn.calls == [(None,)]

    def test_symbol_missing(self, monkeypatch: pytest.MonkeyPatch) -> None:
        fake_cdll(monkeypatch, {"libcrypto.so.3": {}})
        assert _fips._openssl_fips_enabled() is None

    def test_library_missing(self, monkeypatch: pytest.MonkeyPatch) -> None:
        fake_cdll(monkeypatch, {})
        assert _fips._openssl_fips_enabled() is None


class TestRestTls:
    @pytest.fixture
    def fake_ssl(self, monkeypatch: pytest.MonkeyPatch) -> None:
        module = types.ModuleType("_ssl")
        module.__file__ = "/usr/lib64/python3.12/lib-dynload/_ssl.so"
        monkeypatch.setitem(sys.modules, "_ssl", module)

    def _patch(self, monkeypatch: pytest.MonkeyPatch, uses_system: bool | None, fips: bool | None = None) -> None:
        monkeypatch.setattr(_fips, "_uses_system_openssl", lambda path: uses_system)
        monkeypatch.setattr(_fips, "_openssl_fips_enabled", lambda: fips)

    @pytest.mark.usefixtures("linux", "fake_ssl")
    @pytest.mark.parametrize(
        ("uses_system", "fips", "expected"),
        [
            (True, True, "fips"),
            (True, False, "not-fips"),
            (True, None, "unknown"),
            (False, None, "not-fips"),  # e.g. python-build-standalone with OpenSSL compiled in
            (None, None, "unknown"),
        ],
    )
    def test_states(
        self, monkeypatch: pytest.MonkeyPatch, uses_system: bool | None, fips: bool | None, expected: str
    ) -> None:
        self._patch(monkeypatch, uses_system, fips)
        assert _fips._rest_tls() == expected

    @pytest.mark.usefixtures("fake_ssl")
    def test_non_linux_is_unknown(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setattr(_fips.sys, "platform", "darwin")
        self._patch(monkeypatch, True, True)
        assert _fips._rest_tls() == "unknown"

    @pytest.mark.usefixtures("linux")
    def test_builtin_ssl_is_unknown(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setitem(sys.modules, "_ssl", types.ModuleType("_ssl"))
        assert _fips._rest_tls() == "unknown"

    @pytest.mark.usefixtures("linux")
    def test_ssl_missing_is_unknown(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setitem(sys.modules, "_ssl", None)
        assert _fips._rest_tls() == "unknown"


class TestFlightSystemOpenssl:
    @pytest.fixture
    def fake_flight(self, monkeypatch: pytest.MonkeyPatch) -> None:
        pyarrow = types.ModuleType("pyarrow")
        flight_ext = types.ModuleType("pyarrow._flight")
        flight_ext.__file__ = "/site-packages/pyarrow/_flight.cpython-312-x86_64-linux-gnu.so"
        pyarrow._flight = flight_ext  # type: ignore[attr-defined]
        monkeypatch.setitem(sys.modules, "pyarrow", pyarrow)
        monkeypatch.setitem(sys.modules, "pyarrow._flight", flight_ext)

    @pytest.mark.usefixtures("linux", "fake_flight")
    @pytest.mark.parametrize("uses_system", [True, False, None])
    def test_checks_flight_extension(self, monkeypatch: pytest.MonkeyPatch, uses_system: bool | None) -> None:
        paths: list[str] = []

        def uses(path: str) -> bool | None:
            paths.append(path)
            return uses_system

        monkeypatch.setattr(_fips, "_uses_system_openssl", uses)
        assert _fips._flight_system_openssl() is uses_system
        assert paths == ["/site-packages/pyarrow/_flight.cpython-312-x86_64-linux-gnu.so"]

    @pytest.mark.usefixtures("fake_flight")
    def test_non_linux_is_none(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setattr(_fips.sys, "platform", "darwin")
        monkeypatch.setattr(_fips, "_uses_system_openssl", lambda path: True)
        assert _fips._flight_system_openssl() is None


class TestPyarrowHelpers:
    def test_installed(self) -> None:
        pytest.importorskip("pyarrow")
        assert _fips._pyarrow_installed() is True

    def test_not_installed(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setattr(_fips.importlib.util, "find_spec", lambda name: None)
        assert _fips._pyarrow_installed() is False

    def test_spec_error(self, monkeypatch: pytest.MonkeyPatch) -> None:
        def broken(name: str) -> None:
            raise ValueError("pyarrow.__spec__ is None")

        monkeypatch.setattr(_fips.importlib.util, "find_spec", broken)
        assert _fips._pyarrow_installed() is False

    def test_import_error_reported(self, monkeypatch: pytest.MonkeyPatch) -> None:
        real_import = builtins.__import__

        def fake_import(name: str, *args: object, **kwargs: object) -> object:
            if name.split(".")[0] == "pyarrow":
                raise ImportError("libthrift-0.24.0.so: cannot open shared object file: No such file or directory")
            return real_import(name, *args, **kwargs)  # type: ignore[arg-type]

        for mod in [m for m in sys.modules if m.split(".")[0] == "pyarrow"]:
            monkeypatch.delitem(sys.modules, mod)
        monkeypatch.setattr(builtins, "__import__", fake_import)
        error = _fips._pyarrow_import_error()
        assert error is not None
        assert error.startswith("ImportError: libthrift-0.24.0.so")

    def test_import_succeeds(self) -> None:
        pytest.importorskip("pyarrow.flight")
        assert _fips._pyarrow_import_error() is None

    def test_version_from_metadata(self) -> None:
        pa = pytest.importorskip("pyarrow")
        assert _fips._pyarrow_version() == pa.__version__

    def test_version_falls_back_to_module(self, monkeypatch: pytest.MonkeyPatch) -> None:
        def missing(name: str) -> str:
            raise _fips.importlib.metadata.PackageNotFoundError(name)

        monkeypatch.setattr(_fips.importlib.metadata, "version", missing)
        monkeypatch.setitem(sys.modules, "pyarrow", types.SimpleNamespace(__version__="25.0.1"))
        assert _fips._pyarrow_version() == "25.0.1"

    def test_version_unknown(self, monkeypatch: pytest.MonkeyPatch) -> None:
        def missing(name: str) -> str:
            raise _fips.importlib.metadata.PackageNotFoundError(name)

        monkeypatch.setattr(_fips.importlib.metadata, "version", missing)
        monkeypatch.delitem(sys.modules, "pyarrow", raising=False)
        assert _fips._pyarrow_version() is None


class TestFipsStatus:
    def test_exported_from_package(self) -> None:
        assert data_connect_hub.fips_status is fips_status
        assert data_connect_hub.FipsStatus is FipsStatus
        assert {"fips_status", "FipsStatus"} <= set(data_connect_hub.__all__)

    def _patch(self, monkeypatch: pytest.MonkeyPatch, **overrides: Any) -> None:
        defaults: dict[str, Any] = {
            "_host_fips": lambda: True,
            "_rest_tls": lambda: "fips",
            "_pyarrow_installed": lambda: True,
            "_pyarrow_version": lambda: "25.0.1",
            "_pyarrow_import_error": lambda: None,
            "_flight_system_openssl": lambda: True,
        }
        for name, value in {**defaults, **overrides}.items():
            monkeypatch.setattr(_fips, name, value)

    def test_fips_compliant_setup(self, monkeypatch: pytest.MonkeyPatch) -> None:
        self._patch(monkeypatch)
        assert fips_status() == FipsStatus(
            host_fips=True, rest_tls="fips", flight_system_openssl=True, pyarrow_version="25.0.1", pyarrow_error=None
        )

    def test_bundled_pyarrow(self, monkeypatch: pytest.MonkeyPatch) -> None:
        self._patch(monkeypatch, _flight_system_openssl=lambda: False)
        assert fips_status().flight_system_openssl is False

    def test_broken_pyarrow_reported_distinctly(self, monkeypatch: pytest.MonkeyPatch) -> None:
        def unexpected() -> None:
            raise AssertionError("must not inspect a pyarrow that failed to load")

        self._patch(
            monkeypatch,
            _pyarrow_import_error=lambda: "ImportError: libthrift-0.24.0.so",
            _flight_system_openssl=unexpected,
        )
        status = fips_status()
        assert status.flight_system_openssl is None
        assert status.pyarrow_version == "25.0.1"
        assert status.pyarrow_error == "ImportError: libthrift-0.24.0.so"

    def test_pyarrow_not_installed(self, monkeypatch: pytest.MonkeyPatch) -> None:
        def unexpected() -> None:
            raise AssertionError("must not touch a missing pyarrow")

        self._patch(
            monkeypatch,
            _pyarrow_installed=lambda: False,
            _pyarrow_version=unexpected,
            _pyarrow_import_error=unexpected,
            _flight_system_openssl=unexpected,
        )
        status = fips_status()
        assert status.flight_system_openssl is None
        assert status.pyarrow_version is None
        assert status.pyarrow_error is None

    def test_real_environment(self) -> None:
        # Runs against this interpreter and its installed pyarrow: values must be valid.
        status = fips_status()
        assert isinstance(status.host_fips, bool)
        assert status.rest_tls in {"fips", "not-fips", "unknown"}
        assert status.flight_system_openssl in {True, False, None}

    @pytest.mark.skipif(not sys.platform.startswith("linux"), reason="needs Linux")
    def test_real_pyarrow_is_classified(self) -> None:
        # False for PyPI pyarrow (CI), True for Red Hat pyarrow (RHOAI workbench), never None.
        pytest.importorskip("pyarrow.flight")
        assert fips_status().flight_system_openssl in {True, False}

    @pytest.mark.skipif(not sys.platform.startswith("linux"), reason="needs Linux")
    def test_real_python_ssl_is_classified(self) -> None:
        # A Linux CPython with a shared _ssl module always yields a definite answer.
        import _ssl

        if not getattr(_ssl, "__file__", None):
            pytest.skip("_ssl is built into the interpreter")
        assert _fips._uses_system_openssl(_ssl.__file__) in {True, False}

    def test_frozen(self) -> None:
        status = FipsStatus(False, "unknown", None, None, None)
        with pytest.raises(AttributeError):
            status.host_fips = True  # type: ignore[misc]
