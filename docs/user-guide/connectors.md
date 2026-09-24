# 1. Connector

A connector is a data-source integration that enables Data Connect Hub to connect to an external system and read data in a supported format.

| Connector | Unencrypted network transport | TLS-encrypted network transport | Supported ingestion | Input format |
| --- | --- | --- | --- | --- |
| `postgres` | User provides a non-TLS PostgreSQL URI. Example: `postgresql://host/db?sslmode=disable` | User provides a TLS PostgreSQL URI and may set a custom CA via `CA_CERT`. Example: `postgresql://host/db?sslmode=verify-ca` | Tabular | Read-only SQL query. Example: `SELECT id, name FROM users LIMIT 10` |
| `sqlite` | **N/A** — local file access | **N/A** — local file access | Tabular | Read-only SQL query. Example: `SELECT name FROM users LIMIT 10` |
| `elasticsearch` | User provides an `http://` endpoint. Example: `http://host:9200` | User provides an `https://` endpoint and may set a custom CA via `ES_CA_CERT`. Example: `https://host:9200` | Tabular | Elasticsearch Query DSL JSON. The index is specified in the request or connection properties. Example: `{"index":"products","query":{"match_all":{}}}` |
| `milvus` | User provides `MILVUS_URI` with `http://`. Example: `http://host:19530` | User provides `MILVUS_URI` with `https://` and may set a custom CA via `MILVUS_CA_CERT` (optional PEM CA certificate). Example: `https://host:8080` (TLS REST port used by this repository's installation script) | Tabular | Milvus REST API JSON: query, vector search, or get by ID. Example: `{"collectionName":"products","filter":"price > 50"}` |
| `neo4j` | User provides a `neo4j://` URI. Example: `neo4j://host:7687` | User provides a `neo4j+s://` URI and may set a custom CA via `NEO4J_CA_CERT`. Example: `neo4j+s://host:7687` | Tabular | Cypher query. Example: `MATCH (n:Person) RETURN n.name LIMIT 10` |
| `s3` | User sets `AWS_S3_ENDPOINT` to an `http://` endpoint. Example: `http://host:9000` | User provides an HTTPS S3 endpoint and may provide a custom PEM CA via `AWS_S3_CA_CERT`. Example: `https://s3.amazonaws.com` | Binary or tabular | Object path. Supported formats are Parquet, CSV, and JSON Lines; the format is inferred from the extension or `format` property. Example: `data/events.parquet` |
| `uri` | User provides a base URI with `http://`. Example: `http://host/` | User provides a base URI with `https://` and may set a custom CA via `CA_CERT`. Example: `https://host/` | Binary or tabular | GET request JSON. `path` is required and `data_path` is optional. Example: `{"path":"/api/data"}` |

# 2. Connection Type

A connection type defines the credentials and configuration required to connect to a specific data source.

| Connection Type | Source | Provider | Credentials Fields | Notes |
| --- | --- | --- | --- | --- |
| [ElasticSearch](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/elasticsearch.yaml) | Built-in | `elasticsearch` | • `ES_URI`<br>• `ES_USERNAME`<br>• `ES_PASSWORD`<br>• `ES_CA_CERT`<br>• `ES_API_KEY` | — |
| [HuggingFace](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/huggingface.yaml) | Built-in | `huggingface` | • `HF_URI`<br>• `HF_TOKEN` | — |
| [Milvus](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/milvus.yaml) | Built-in | `milvus` | • `MILVUS_URI`<br>• `MILVUS_TOKEN`<br>• `MILVUS_DATABASE`<br>• `MILVUS_CA_CERT` | — |
| [Neo4j](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/neo4j.yaml) | Built-in | `neo4j` | • `NEO4J_URI`<br>• `NEO4J_USERNAME`<br>• `NEO4J_PASSWORD`<br>• `NEO4J_DATABASE`<br>• `NEO4J_CA_CERT` | — |
| [PGVector](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/pgvector.yaml) | Built-in | `postgres` | • `URI`<br>• `CA_CERT` | — |
| [Postgres](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/postgres.yaml) | Built-in | `postgres` | • `URI`<br>• `CA_CERT` | — |
| [URI](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/connection-types/uri.yaml) | Built-in | `uri` | • `URI`<br>• `TOKEN`<br>• `USERNAME`<br>• `PASSWORD`<br>• `CA_CERT` | — |
| `s3` | Imported from RHOAI | `s3` | • `AWS_ACCESS_KEY_ID`<br>• `AWS_SECRET_ACCESS_KEY`<br>• `AWS_S3_ENDPOINT`<br>• `AWS_DEFAULT_REGION`<br>• `AWS_S3_BUCKET` | — |
| `oci-v1` | Imported from RHOAI | `oci-v1` | • `ACCESS_TYPE`<br>• `OCI_HOST` | No matching DCH connector is currently available. |
| `uri-v1` | Imported from RHOAI | `uri-v1` | • `URI` | No matching DCH connector is currently available. |
