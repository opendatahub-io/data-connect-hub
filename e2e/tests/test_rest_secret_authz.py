"""Requester authorization on credentials_ref secrets.

The services resolve a connection's credentials_ref secret with their own
service account, so they must first verify the requesting user could read
that secret themselves. These tests pin the deny/allow behavior: a secret
outside the user's RBAC is rejected with 403, a granted secret is accepted,
and changing a secret_ref to an ungranted secret is rejected.
"""

from __future__ import annotations

import uuid

import pytest
from data_connect_hub import (
    CredentialField,
    CredentialsRef,
    DataConnectClient,
    DCHForbiddenError,
)


@pytest.fixture()
def pg_connection_type(create_connection_type):
    return create_connection_type(
        provider="postgres",
        credentials_fields=[
            CredentialField(
                name="URI",
                label="URI",
                required=True,
                type="string",
            )
        ],
    )


def _ungranted_secret() -> str:
    # A secret name outside every RBAC grant in the tenant namespace. The
    # authorization check is pure RBAC evaluation, so the secret need not
    # exist: referencing it must still be denied.
    return f"dch-e2e-forbidden-{uuid.uuid4().hex[:8]}"


def _require_user_secret(user_secret: str | None) -> str:
    if not user_secret:
        pytest.skip("DCH_USER_SECRET not set (run e2e/run-e2e.sh first)")
    return user_secret


class TestRestSecretAuthz:
    def test_create_connection_with_ungranted_secret_ref_returns_403(
        self,
        rest_client: DataConnectClient,
        pg_connection_type,
    ) -> None:
        with pytest.raises(DCHForbiddenError) as exc_info:
            rest_client.create_connection(
                name="e2e-authz-denied-pg",
                connection_type_id=pg_connection_type.id,
                data_format="tabular",
                credentials_ref=CredentialsRef(secret=_ungranted_secret()),
                properties={},
            )

        assert exc_info.value.status_code == 403
        assert "referenced secret" in exc_info.value.body

    def test_create_connection_with_granted_secret_ref_succeeds(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
        user_secret: str | None,
    ) -> None:
        user_secret = _require_user_secret(user_secret)
        conn = create_connection(
            name="e2e-authz-allowed-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=CredentialsRef(secret=user_secret),
        )

        fetched = rest_client.get_connection(conn.id)
        assert fetched.credentials_ref.secret == user_secret

    def test_update_connection_changing_secret_ref_to_ungranted_returns_403(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
        user_secret: str | None,
    ) -> None:
        user_secret = _require_user_secret(user_secret)
        conn = create_connection(
            name="e2e-authz-update-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=CredentialsRef(secret=user_secret),
        )

        with pytest.raises(DCHForbiddenError) as exc_info:
            rest_client.update_connection(
                conn.id,
                credentials_ref=CredentialsRef(secret=_ungranted_secret()),
            )

        assert exc_info.value.status_code == 403

        # The original connection is untouched by the rejected update.
        fetched = rest_client.get_connection(conn.id)
        assert fetched.credentials_ref.secret == user_secret

    def test_update_connection_untouched_secret_ref_succeeds(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
        user_secret: str | None,
    ) -> None:
        user_secret = _require_user_secret(user_secret)
        conn = create_connection(
            name="e2e-authz-untouched-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=CredentialsRef(secret=user_secret),
        )

        updated = rest_client.update_connection(conn.id, name="e2e-authz-renamed-pg")
        assert updated.name == "e2e-authz-renamed-pg"
        assert updated.credentials_ref.secret == user_secret
