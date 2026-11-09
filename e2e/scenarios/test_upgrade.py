"""Upgrade scenario: data created before an upgrade must survive it.

Driven by hack/ci/upgrade-test.sh in two phases:

    DCH_UPGRADE_PHASE=pre  — create a connection type and connection, run a
                             Flight query, and persist identifiers and counts
                             to DCH_UPGRADE_STATE_FILE.
    DCH_UPGRADE_PHASE=post — verify the pre-upgrade resources are intact and
                             the same query returns the same rows.

Resources created in the pre phase are deliberately not cleaned up: their
survival across the upgrade is the point of the test.
"""

from __future__ import annotations

import json
import os
import uuid
from pathlib import Path

import pytest

from data_connect_hub import CredentialsRef, DataConnectClient

QUERY = "SELECT * FROM dch_e2e.cities ORDER BY id"


def _require_phase(phase: str) -> None:
    if os.getenv("DCH_UPGRADE_PHASE") != phase:
        pytest.skip(f"DCH_UPGRADE_PHASE is not '{phase}'")


def _state_file() -> Path:
    path = os.getenv("DCH_UPGRADE_STATE_FILE")
    if not path:
        pytest.fail("DCH_UPGRADE_STATE_FILE is not set")
    return Path(path)


@pytest.fixture(scope="module")
def upgrade_state() -> dict:
    path = _state_file()
    if not path.exists():
        pytest.fail(f"upgrade state file not found: {path} (run the pre phase first)")
    return json.loads(path.read_text())


class TestUpgrade:
    def test_pre_upgrade_seed(
        self,
        dch_client: DataConnectClient,
        rest_client: DataConnectClient,
        pg_secret: str | None,
    ) -> None:
        _require_phase("pre")
        if not pg_secret:
            pytest.skip("DCH_PG_SECRET not set (run e2e/run-e2e.sh first)")

        suffix = uuid.uuid4().hex[:8]
        connection_type = rest_client.create_connection_type(
            name=f"e2e-upgrade-type-{suffix}",
            provider="postgres",
            description="upgrade test connection type",
        )
        connection = rest_client.create_connection(
            name=f"e2e-upgrade-conn-{suffix}",
            connection_type_id=connection_type.id,
            data_format="tabular",
            credentials_ref=CredentialsRef(secret=pg_secret),
            properties={},
        )

        table = dch_client.read(QUERY, connection.id)
        assert table.num_rows > 0

        state = {
            "connection_type_id": connection_type.id,
            "connection_id": connection.id,
            "connection_name": connection.name,
            "connection_type_count": len(rest_client.list_connection_types()),
            "rows": table.num_rows,
            "cities": table.column("name").to_pylist(),
        }
        _state_file().write_text(json.dumps(state, indent=2))
        print(f"Seeded upgrade state: {json.dumps(state, indent=2)}")

    def test_post_upgrade_connection_types_preserved(
        self,
        rest_client: DataConnectClient,
        upgrade_state: dict,
    ) -> None:
        _require_phase("post")
        types = rest_client.list_connection_types()
        assert len(types) >= upgrade_state["connection_type_count"]
        assert any(t.id == upgrade_state["connection_type_id"] for t in types)

    def test_post_upgrade_connection_survives(
        self,
        rest_client: DataConnectClient,
        upgrade_state: dict,
    ) -> None:
        _require_phase("post")
        connection = rest_client.get_connection(upgrade_state["connection_id"])
        assert connection.id == upgrade_state["connection_id"]
        assert connection.name == upgrade_state["connection_name"]

    def test_post_upgrade_flight_query_returns_same_data(
        self,
        dch_client: DataConnectClient,
        upgrade_state: dict,
    ) -> None:
        _require_phase("post")
        table = dch_client.read(QUERY, upgrade_state["connection_id"])
        assert table.num_rows == upgrade_state["rows"]
        assert table.column("name").to_pylist() == upgrade_state["cities"]
