"""Connection CRUD via REST API."""

from __future__ import annotations

import json
import uuid

import pytest
from data_connect_hub import (
    CredentialField,
    CredentialsRef,
    DataConnectClient,
    DCHNotFoundError,
    DCHValidationError,
    InlineCredentials,
)


@pytest.fixture()
def pg_connection_type(create_connection_type):
    return create_connection_type(provider="postgres")


@pytest.fixture()
def connection_credentials_ref(pg_secret: str | None) -> CredentialsRef:
    if not pg_secret:
        pytest.skip("DCH_PG_SECRET is not configured")
    return CredentialsRef(secret=pg_secret)


class TestRestConnection:
    @pytest.mark.parametrize("credentials_kind", ["ref", "inline"])
    def test_crud(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
        cleanup_secrets,
        pg_secret: str | None,
        credentials_kind: str,
    ) -> None:
        expected_name = f"e2e-my-pg-{credentials_kind}"
        properties = {"database": "testdb"}

        if credentials_kind == "ref":
            if not pg_secret:
                pytest.skip("DCH_PG_SECRET is not configured")
            credentials_ref = CredentialsRef(secret=pg_secret)
            credentials = None
        else:
            secret_name = f"e2e-inline-crud-{uuid.uuid4().hex}"
            cleanup_secrets(secret_name)
            credentials_ref = None
            credentials = InlineCredentials(
                secret=secret_name,
                properties={"E2E_TEST_VALUE": "e2e-test-value"},
            )

        conn = create_connection(
            name=expected_name,
            connection_type_id=pg_connection_type.id,
            credentials_ref=credentials_ref,
            credentials=credentials,
            properties=properties,
        )

        connections = rest_client.list_connections()
        assert conn.id in [c.id for c in connections]

        fetched = rest_client.get_connection(conn.id)
        assert fetched.name == expected_name
        assert fetched.data_connection_type_id == pg_connection_type.id
        assert fetched.properties["database"] == "testdb"

    def test_create_with_credentials_ref_missing_secret_returns_404(
        self,
        rest_client: DataConnectClient,
        pg_connection_type,
    ) -> None:
        secret_name = f"e2e-missing-{uuid.uuid4().hex}"

        with pytest.raises(DCHNotFoundError) as exc_info:
            rest_client.create_connection(
                name="e2e-missing-secret",
                connection_type_id=pg_connection_type.id,
                data_format="tabular",
                credentials_ref=CredentialsRef(secret=secret_name),
            )

        assert exc_info.value.status_code == 404

    def test_create_with_credentials_ref_missing_required_field_returns_400(
        self,
        rest_client: DataConnectClient,
        create_connection_type,
        pg_secret: str | None,
    ) -> None:
        if not pg_secret:
            pytest.skip("DCH_PG_SECRET is not configured")

        required_field = f"e2e-required-{uuid.uuid4().hex}"
        connection_type = create_connection_type(
            provider="postgres",
            credentials_fields=[
                CredentialField(
                    name=required_field,
                    label="Required E2E field",
                    required=True,
                    type="string",
                )
            ],
        )

        with pytest.raises(DCHValidationError) as exc_info:
            rest_client.create_connection(
                name="e2e-incomplete-secret",
                connection_type_id=connection_type.id,
                data_format="tabular",
                credentials_ref=CredentialsRef(secret=pg_secret),
            )

        assert exc_info.value.status_code == 400
        body = json.loads(exc_info.value.body)
        assert body["code"] == "credentials_check_failed"
        assert body["message"] == f"Required field {required_field} is missing"

    def test_create_with_inline_credentials_returns_201(
        self,
        cleanup_secrets,
        create_connection,
        pg_connection_type,
    ) -> None:
        secret_name = f"e2e-inline-{uuid.uuid4().hex}"
        cleanup_secrets(secret_name)

        conn = create_connection(
            name="e2e-inline-success",
            connection_type_id=pg_connection_type.id,
            credentials=InlineCredentials(
                secret=secret_name,
                properties={"E2E_TEST_VALUE": "e2e-test-value"},
            ),
        )

        assert conn.name == "e2e-inline-success"
        assert conn.credentials_ref.secret == secret_name

    def test_create_with_inline_credentials_missing_required_field_returns_400(
        self,
        rest_client: DataConnectClient,
        create_connection_type,
    ) -> None:
        required_field = f"e2e-inline-required-{uuid.uuid4().hex}"
        connection_type = create_connection_type(
            provider="postgres",
            credentials_fields=[
                CredentialField(
                    name=required_field,
                    label="Required inline E2E field",
                    required=True,
                    type="string",
                )
            ],
        )

        with pytest.raises(DCHValidationError) as exc_info:
            rest_client.create_connection(
                name="e2e-inline-incomplete-credentials",
                connection_type_id=connection_type.id,
                data_format="tabular",
                credentials=InlineCredentials(
                    secret=f"e2e-inline-{uuid.uuid4().hex}",
                    properties={"UNRELATED": "value"},
                ),
            )

        assert exc_info.value.status_code == 400
        body = json.loads(exc_info.value.body)
        assert body["code"] == "credentials_check_failed"
        assert body["message"] == f"Required field {required_field} is missing"

    def test_delete(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
        connection_credentials_ref,
    ) -> None:
        conn = create_connection(
            name="e2e-delete-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=connection_credentials_ref,
        )
        rest_client.delete_connection(conn.id)

        with pytest.raises(DCHNotFoundError):
            rest_client.get_connection(conn.id)

    def test_get_nonexistent_returns_404(self, rest_client: DataConnectClient) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHNotFoundError):
            rest_client.get_connection(fake_id)

    def test_delete_nonexistent_returns_404(self, rest_client: DataConnectClient) -> None:
        fake_id = str(uuid.uuid4())
        with pytest.raises(DCHNotFoundError):
            rest_client.delete_connection(fake_id)

    def test_update(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
        connection_credentials_ref,
    ) -> None:
        conn = create_connection(
            name="e2e-update-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=connection_credentials_ref,
            properties={"database": "testdb"},
        )
        updated = rest_client.update_connection(conn.id, name="e2e-updated-pg")
        assert updated.name == "e2e-updated-pg"
