"""Connection CRUD via REST API."""

from __future__ import annotations

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


class TestRestConnection:
    def _create_and_assert_connection(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
        *,
        credentials_ref: CredentialsRef | None = None,
        credentials: InlineCredentials | None = None,
        expected_secret: str,
    ) -> None:
        conn = create_connection(
            name="e2e-crud-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=credentials_ref,
            credentials=credentials,
            properties={"database": "testdb"},
        )

        connections = rest_client.list_connections()
        assert conn.id in [c.id for c in connections]

        fetched = rest_client.get_connection(conn.id)
        assert fetched.name == "e2e-crud-pg"
        assert fetched.data_connection_type_id == pg_connection_type.id
        assert fetched.properties["database"] == "testdb"
        assert fetched.credentials_ref.secret == expected_secret

    def test_crud_with_secret_ref(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
    ) -> None:
        credentials_ref = CredentialsRef(secret="e2e-ns/e2e-secret")
        self._create_and_assert_connection(
            rest_client,
            create_connection,
            pg_connection_type,
            credentials_ref=credentials_ref,
            expected_secret=credentials_ref.secret,
        )

    def test_crud_with_inline_credentials(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
    ) -> None:
        credentials = InlineCredentials(
            secret=f"e2e-inline-{uuid.uuid4().hex[:8]}",
            properties={"URI": "postgresql://e2e-user:e2e-password@localhost:5432/e2e"},
        )
        self._create_and_assert_connection(
            rest_client,
            create_connection,
            pg_connection_type,
            credentials=credentials,
            expected_secret=credentials.secret,
        )

    def test_create_with_inline_credentials_rejects_missing_required_field(
        self,
        rest_client: DataConnectClient,
        pg_connection_type,
    ) -> None:
        credentials = InlineCredentials(
            secret=f"e2e-inline-invalid-{uuid.uuid4().hex[:8]}",
            properties={},
        )

        with pytest.raises(DCHValidationError) as exc_info:
            rest_client.create_connection(
                name="e2e-invalid-inline-pg",
                connection_type_id=pg_connection_type.id,
                data_format="tabular",
                credentials=credentials,
                properties={},
            )

        assert exc_info.value.status_code == 400
        assert "credentials_check_failed" in exc_info.value.body
        assert "Required field URI is missing" in exc_info.value.body

    def test_delete(
        self,
        rest_client: DataConnectClient,
        create_connection,
        pg_connection_type,
    ) -> None:
        conn = create_connection(
            name="e2e-delete-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=CredentialsRef(secret="e2e-ns/e2e-secret"),
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
    ) -> None:
        conn = create_connection(
            name="e2e-update-pg",
            connection_type_id=pg_connection_type.id,
            credentials_ref=CredentialsRef(secret="e2e-ns/e2e-secret"),
            properties={"database": "testdb"},
        )
        updated = rest_client.update_connection(conn.id, name="e2e-updated-pg")
        assert updated.name == "e2e-updated-pg"
