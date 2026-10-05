# 1. Connector

A connector is a data-source integration that enables Data Connect Hub to connect to an external system and read data in a supported format.

> **Note**: Data Connect Hub does not currently ship a build that performs its cryptography with FIPS 140-3 validated modules. The **FIPS-mode deployment** column records the expected status of each connector in the planned FIPS build variant — it does not describe the images available today.

| Connector | Unencrypted network transport | TLS-encrypted network transport | Supported ingestion | Input format | FIPS-mode deployment |
| --- | --- | --- | --- | --- | --- |
| `postgres` | User provides a non-TLS PostgreSQL URI. Example: `postgresql://host/db?sslmode=disable` | User provides a TLS PostgreSQL URI and may set a custom CA via `CA_CERT`. Example: `postgresql://host/db?sslmode=verify-ca` | Tabular | • **Input:** Read-only SQL query.<br>• **Example:** `SELECT id, name FROM users LIMIT 10` | Under evaluation — password authentication depends on digest algorithms that may not be available in FIPS mode; client-certificate authentication is the likely requirement |
| `sqlite` | **N/A** — local file access | **N/A** — local file access | Tabular | • **Input:** Read-only SQL query.<br>• **Example:** `SELECT name FROM users LIMIT 10` | Planned — local file access performs no cryptography |
| `elasticsearch` | User provides an `http://` endpoint. Example: `http://host:9200` | User provides an `https://` endpoint and may set a custom CA via `CA_CERT`. Example: `https://host:9200` | Tabular | • **Input:** Elasticsearch search API JSON body. `index` selects the target index (or set a default in connection properties).<br>• **Example:** `{"index":"products","query":{"match_all":{}}}` | Planned — TLS is the only cryptography involved |
| `milvus` | User provides `URI` with `http://`. Example: `http://host:19530` | User provides `URI` with `https://` and may set a custom CA via `CA_CERT` (optional PEM CA certificate). Example: `https://host:8080` (TLS REST port used by this repository's installation script) | Tabular | • **Input:** Milvus REST API JSON: query, vector search, or get by ID.<br>• **Example:** `{"collectionName":"products","filter":"price > 50"}` | Planned — TLS is the only cryptography involved |
| `neo4j` | User provides a `neo4j://` URI. Example: `neo4j://host:7687` | User provides a `neo4j+s://` URI and may set a custom CA via `CA_CERT`. Example: `neo4j+s://host:7687` | Tabular | • **Input:** Cypher query.<br>• **Example:** `MATCH (n:Person) RETURN n.name LIMIT 10` | Under evaluation — the Neo4j driver selects its own TLS implementation, which Data Connect Hub cannot currently redirect to the system modules |
| `s3` | User sets `AWS_S3_ENDPOINT` to an `http://` endpoint. Example: `http://host:9000` | User provides an HTTPS S3 endpoint and may provide a custom PEM CA via `AWS_S3_CA_CERT`. Example: `https://s3.amazonaws.com` | Binary or tabular | • **Input:** S3 object path.<br>• **Tabular formats:** Parquet, CSV, JSON, JSONL — inferred from file extension unless the connection's `format` property is set.<br>• **Example:** `data/events.parquet` | Under evaluation — S3 request signing uses digest algorithms that are not FIPS-approved |
| `uri` | User provides a base URI with `http://`. Example: `http://host/` | User provides a base URI with `https://` and may set a custom CA via `CA_CERT`. Example: `https://host/` | Binary or tabular | • **Input:** JSON with `path` (required). Optional: `data_path` (nested-array selector for JSON responses), `format` (override auto-detection).<br>• **Tabular formats:** Parquet, CSV, JSON, JSONL — inferred from response `Content-Type` header, then file extension, unless `format` is set.<br>• **Example:** `{"path":"/api/data"}` | Under evaluation — shares the object-storage request-signing stack used by `s3` |

# 2. Connection Type

A connection type defines the credentials and configuration required to connect to a specific data source.

> **Note**: In the table below, **Provider** identifies the Data Connect Hub connector used by each connection type. See the Connector table above for supported connectors.

| Connection Type | Source | Provider | Credentials Fields | Notes |
| --- | --- | --- | --- | --- |
| [ElasticSearch](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/elasticsearch.yaml) | Built-in | `elasticsearch` | • `URI`<br>• `USERNAME`<br>• `PASSWORD`<br>• `CA_CERT`<br>• `API_KEY` | — |
| [HuggingFace](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/huggingface.yaml) | Built-in | `huggingface` | • `URI`<br>• `TOKEN` | No DCH connector is currently available for the `huggingface` provider. |
| [Milvus](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/milvus.yaml) | Built-in | `milvus` | • `URI`<br>• `TOKEN`<br>• `DATABASE`<br>• `CA_CERT` | — |
| [Neo4j](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/neo4j.yaml) | Built-in | `neo4j` | • `URI`<br>• `USERNAME`<br>• `PASSWORD`<br>• `DATABASE`<br>• `CA_CERT` | — |
| [PGVector](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/pgvector.yaml) | Built-in | `postgres` | • `URI`<br>• `CA_CERT` | — |
| [Postgres](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/postgres.yaml) | Built-in | `postgres` | • `URI`<br>• `CA_CERT` | — |
| [URI](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/uri.yaml) | Built-in | `uri` | • `URI`<br>• `TOKEN`<br>• `USERNAME`<br>• `PASSWORD`<br>• `CA_CERT` | — |
| [s3](https://github.com/opendatahub-io/odh-dashboard/blob/main/manifests/base/connection-types/s3.yaml) | Imported from RHOAI | `s3` | • `AWS_ACCESS_KEY_ID`<br>• `AWS_SECRET_ACCESS_KEY`<br>• `AWS_S3_ENDPOINT`<br>• `AWS_DEFAULT_REGION`<br>• `AWS_S3_BUCKET` | For a custom TLS CA, include the optional `AWS_S3_CA_CERT` key in the connection Secret. |
| [oci-v1](https://github.com/opendatahub-io/odh-dashboard/blob/main/manifests/base/connection-types/oci-v1.yaml) | Imported from RHOAI | `oci-v1` | • `ACCESS_TYPE`<br>• `OCI_HOST` | No DCH connector is currently available for the `oci-v1` provider. |
| [uri-v1](https://github.com/opendatahub-io/odh-dashboard/blob/main/manifests/base/connection-types/uri-v1.yaml) | Imported from RHOAI | `uri-v1` | • `URI` | No DCH connector is currently available for the `uri-v1` provider. |
