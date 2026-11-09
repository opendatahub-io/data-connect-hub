"""Keep constraints-min.txt pinned to the lower bounds declared in pyproject.toml.

`make sdk-test-min` installs with these pins. A pin above the declared lower bound
would make that job test a newer version than users may have, so this test
fails on any drift.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest
from packaging.requirements import Requirement
from packaging.utils import canonicalize_name
from packaging.version import Version

_SDK_DIR = Path(__file__).resolve().parent.parent
_PYPROJECT = _SDK_DIR / "pyproject.toml"
_CONSTRAINTS = _SDK_DIR / "constraints-min.txt"


def _runtime_requirements() -> list[Requirement]:
    project = tomllib.loads(_PYPROJECT.read_text())["project"]
    specs = [*project["dependencies"], *project["optional-dependencies"]["flight"]]
    return [Requirement(s) for s in specs]


def _constraint_pins() -> dict[str, Version]:
    pins: dict[str, Version] = {}
    for raw in _CONSTRAINTS.read_text().splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        req = Requirement(line)
        (spec,) = req.specifier
        assert spec.operator == "==", f"constraints-min.txt must pin with '==': {line}"
        pins[canonicalize_name(req.name)] = Version(spec.version)
    return pins


@pytest.mark.skipif(not _CONSTRAINTS.is_file(), reason="constraints-min.txt is not shipped in the sdist")
def test_constraints_pin_declared_lower_bounds() -> None:
    pins = _constraint_pins()
    for req in _runtime_requirements():
        lower = [Version(s.version) for s in req.specifier if s.operator == ">="]
        assert len(lower) == 1, f"{req} must declare exactly one '>=' lower bound"
        name = canonicalize_name(req.name)
        assert name in pins, f"{req.name} is missing from constraints-min.txt"
        assert pins[name] == lower[0], (
            f"constraints-min.txt pins {req.name}=={pins[name]} but pyproject.toml declares >={lower[0]}"
        )
