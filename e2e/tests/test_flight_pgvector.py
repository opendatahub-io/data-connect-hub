"""pgvector types through Flight SQL via the out-of-box PGVector connection type.

Requires run-e2e.sh to have seeded dch_e2e.embeddings (only when the tenant
PostgreSQL has pgvector; hack/install-postgresql.sh --pgvector installs it)
and set DCH_PG_SECRET.
"""

from __future__ import annotations

import pyarrow as pa
import pytest
from data_connect_hub import DataConnectClient, DCHQueryError

TABLE = "dch_e2e.embeddings"


@pytest.fixture(scope="module")
def pgvector_connection(dch_client: DataConnectClient, pgvector_flight_connection: str) -> str:
    """Skip the module when the tenant PostgreSQL was seeded without pgvector."""
    seeded = dch_client.read(
        "SELECT 1 FROM pg_tables WHERE schemaname = 'dch_e2e' AND tablename = 'embeddings'",
        pgvector_flight_connection,
    )
    if seeded.num_rows == 0:
        pytest.skip(f"{TABLE} not seeded (tenant PostgreSQL lacks the pgvector extension)")
    return pgvector_flight_connection


def _is_float32_list(t: pa.DataType) -> bool:
    return pa.types.is_list(t) and pa.types.is_float32(t.value_type)


class TestFlightPgvector:
    def test_vector_columns_schema(self, dch_client: DataConnectClient, pgvector_connection: str) -> None:
        table = dch_client.read(f"SELECT * FROM {TABLE} ORDER BY id", pgvector_connection)
        assert table.num_rows == 3

        for col in ("embedding", "half_embedding"):
            col_type = table.schema.field(col).type
            assert _is_float32_list(col_type), f"{col}: expected list<float32>, got {col_type}"
        assert pa.types.is_string(table.schema.field("sparse_embedding").type)

    def test_vector_values(self, dch_client: DataConnectClient, pgvector_connection: str) -> None:
        table = dch_client.read(f"SELECT embedding FROM {TABLE} ORDER BY id", pgvector_connection)
        rows = table.column("embedding").to_pylist()
        assert rows[0] == pytest.approx([0.1, 0.2, 0.3], rel=1e-6)
        assert rows[1] == pytest.approx([1.0, -2.0, 3.5])
        assert rows[2] is None

    def test_halfvec_values(self, dch_client: DataConnectClient, pgvector_connection: str) -> None:
        table = dch_client.read(f"SELECT half_embedding FROM {TABLE} ORDER BY id", pgvector_connection)
        assert table.column("half_embedding").to_pylist() == [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], None]

    def test_sparsevec_values(self, dch_client: DataConnectClient, pgvector_connection: str) -> None:
        table = dch_client.read(f"SELECT sparse_embedding FROM {TABLE} ORDER BY id", pgvector_connection)
        assert table.column("sparse_embedding").to_pylist() == ["{1:1.5,3:2.5}/5", "{2:9}/5", None]

    def test_similarity_search(self, dch_client: DataConnectClient, pgvector_connection: str) -> None:
        table = dch_client.read(
            f"SELECT id, embedding, embedding <-> '[0,0,0]' AS distance "
            f"FROM {TABLE} WHERE embedding IS NOT NULL "
            "ORDER BY embedding <-> '[0,0,0]' LIMIT 2",
            pgvector_connection,
        )
        assert table.column("id").to_pylist() == [1, 2]
        assert _is_float32_list(table.schema.field("embedding").type)
        # L2 norms of [0.1,0.2,0.3] and [1,-2,3.5] (float32 inputs).
        assert table.column("distance").to_pylist() == pytest.approx([0.3741657, 4.1533119], rel=1e-5)

    @pytest.mark.parametrize(
        ("op", "expected_ids", "expected_distances"),
        [
            # Cosine distance. A zero query vector would yield NaN, hence [1,1,1].
            ("<=>", [1, 2], [0.0741799, 0.6524760]),
            # Negative inner product: the ordering reverses relative to <-> and <=>.
            ("<#>", [2, 1], [-2.5, -0.6]),
        ],
    )
    def test_similarity_operators(
        self,
        dch_client: DataConnectClient,
        pgvector_connection: str,
        op: str,
        expected_ids: list[int],
        expected_distances: list[float],
    ) -> None:
        table = dch_client.read(
            f"SELECT id, embedding {op} '[1,1,1]' AS distance "
            f"FROM {TABLE} WHERE embedding IS NOT NULL "
            f"ORDER BY embedding {op} '[1,1,1]'",
            pgvector_connection,
        )
        assert table.column("id").to_pylist() == expected_ids
        assert table.column("distance").to_pylist() == pytest.approx(expected_distances, rel=1e-5)

    def test_write_is_rejected(self, dch_client: DataConnectClient, pgvector_connection: str) -> None:
        with pytest.raises(DCHQueryError, match=r"(?i)read-only"):
            dch_client.read(f"DELETE FROM {TABLE}", pgvector_connection)

    def test_get_tables_schema(self, dch_client: DataConnectClient, pgvector_connection: str) -> None:
        table = dch_client.get_tables(pgvector_connection, table_name_filter="embeddings", include_schema=True)
        assert table.num_rows == 1

        schema_bytes = table.column("table_schema")[0].as_py()
        schema = pa.ipc.read_schema(pa.BufferReader(schema_bytes))
        assert _is_float32_list(schema.field("embedding").type)
        assert _is_float32_list(schema.field("half_embedding").type)
        assert pa.types.is_string(schema.field("sparse_embedding").type)
