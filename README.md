# Data Connect Hub

Data Connect Hub (DCH) is a middleware service that provides a single integration point for ingesting data from multiple heterogeneous data sources. Rather than each consuming service maintaining its own client library stacks for S3, HDFS, NFS, relational databases, and others, DCH centralises connection metadata management and exposes data through two purpose-built APIs.

## API docs

https://opendatahub-io.github.io/data-connect-hub/

## Workspace Layout

```
data-connect-hub/
├── services/
│   ├── flight/                Arrow Flight gRPC service (binary)
│   └── rest/                  HTTP REST service (binary)
├── connectors/
│   ├── elasticsearch/         Elasticsearch data reader (library)
│   ├── milvus/                Milvus data reader (library)
│   ├── neo4j/                 Neo4j data reader (library)
│   ├── postgres/              PostgreSQL data reader (library)
│   ├── s3/                    S3 data reader (library)
│   ├── sqlite/                SQLite data reader (library)
│   └── uri/                   URI data reader (library)
├── libs/
│   ├── commons/               Shared types and traits
│   ├── pg-meta-store/         PostgreSQL metadata store
│   └── kube-utils/            Kubernetes utility helpers
├── dc-controller/             Go-based ODH operator controller
├── sdk/
│   └── python/                Python SDK (REST client)
├── config/                    Kustomize deployment configs
├── hack/                      Scripts and Python tooling
├── docs/                      Documentation and proposals
├── Cargo.toml                 Workspace manifest
├── Makefile                   Build, test, lint, container targets
├── clippy.toml                Clippy configuration
├── rustfmt.toml               Rustfmt configuration
└── rust-toolchain.toml        Rust toolchain pinning
```

## Prerequisites

- Rust 1.96+
- PostgreSQL (for metadata storage and integration testing)
- Podman or Docker (for container builds)

## Getting Started

```sh
# Build all crates
make build

# Run all tests
make test

# Format and lint
make fmt
make lint
```

### Pre-commit Hooks

This repository uses [pre-commit](https://pre-commit.com/) to run local checks before each commit.

```sh
# Install pre-commit (example with pipx)
pipx install pre-commit

# Install repository hooks
make setup-hooks
```

The configured hooks run Rust formatting checks (`cargo fmt --check`), workspace compilation checks (`cargo check`), and Clippy (`-D warnings`).

### Running the Services

Both services need a PostgreSQL database for metadata storage and a TOML
config file.  Keep your local configs in the gitignored `.local/` directory.

#### 1. Set up PostgreSQL

You can use a local installation or run PostgreSQL in a container.

**Option A — Local PostgreSQL**

```sh
createuser -s dch_user
createdb -O dch_user dch_db
psql -d dch_db -c "ALTER USER dch_user WITH PASSWORD 'dch_password';"
```

**Option B — Podman / Docker**

```sh
podman run -d --name dch-postgres \
  -e POSTGRES_USER=dch_user \
  -e POSTGRES_PASSWORD=dch_password \
  -e POSTGRES_DB=dch_db \
  -p 5432:5432 \
  postgres:17
```

The services create the required tables automatically on startup — no manual
schema step is needed.

#### 2. Create local config files

Create a `.local/` directory at the repository root (already gitignored).

**`.local/rest-config.toml`**

```toml
[database]
url = "postgresql://dch_user:dch_password@localhost:5432/dch_db"

[server]
address = "127.0.0.1"
port = 8080

[global-connection-types]
tenant-id = "default"
```

**`.local/flight-config.toml`**

```toml
[database]
url = "postgresql://dch_user:dch_password@localhost:5432/dch_db"

[server]
address = "127.0.0.1"
port = 50051

[ingestion_cache_pools]
max_capacity = 50
ttl_secs = 30
idle_secs = 30

[query]
batch_size = 512
max_rows = 1000000

[global-connection-types]
tenant-id = "default"

[tls]
# cert_file = "/etc/tls/private/tls.crt"
# key_file = "/etc/tls/private/tls.key"

[auth]
enabled = false

[metrics]
enabled = false

[connectors.default]
connection_timeout_secs = 10

# Per-connector overrides (optional, inherits from [connectors.default]):
# [connectors.postgres]
# connection_timeout_secs = 30
```

#### 3. Start a service

```sh
# REST service (HTTP :8080)
cargo run -p rest-service -- --config .local/rest-config.toml

# Flight service (gRPC :50051)
cargo run -p flight-service -- --config .local/flight-config.toml
```

#### 4. Verify it works

```sh
# REST service
curl http://localhost:8080/health
# {"service":"rest-service"}

curl http://localhost:8080/api/v1alpha1/data/connections
# []
```

For the Flight service, use any gRPC client (e.g.
[grpcurl](https://github.com/fullstorydev/grpcurl),
[Postman](https://www.postman.com/),
[Evans](https://github.com/ktr0731/evans)) to call the
health check:

```sh
# Example with grpcurl
grpcurl -plaintext localhost:50051 grpc.health.v1.Health/Check
# {"status":"SERVING"}
```

## REST API

| Method | Path                                                              | Description                                    |
| ------ | ----------------------------------------------------------------- | ---------------------------------------------- |
| GET    | `/health`                                                         | Health check                                   |
| GET    | `/api/v1alpha1/data/connections`                                  | List all connections                           |
| POST   | `/api/v1alpha1/data/connections`                                  | Create a connection                            |
| GET    | `/api/v1alpha1/data/connections/{id}`                             | Get a connection                               |
| PATCH  | `/api/v1alpha1/data/connections/{id}`                             | Update a connection                            |
| DELETE | `/api/v1alpha1/data/connections/{id}`                             | Delete a connection                            |
| GET    | `/api/v1alpha1/data/connections/{id}/binary`                      | Get ingestion data (not implemented)           |
| POST   | `/api/v1alpha1/data/connections/{id}/readiness`                   | Audit an existing connection                   |
| PUT    | `/api/v1alpha1/data/connections/{id}/exports/secrets/{secret_name}` | Export connection credentials to a K8s secret |
| GET    | `/api/v1alpha1/data/connection-types`                             | List all connection types                      |
| POST   | `/api/v1alpha1/data/connection-types`                             | Create a connection type                       |
| GET    | `/api/v1alpha1/data/connection-types/{id}`                        | Get a connection type                          |
| PATCH  | `/api/v1alpha1/data/connection-types/{id}`                        | Update a connection type                       |
| DELETE | `/api/v1alpha1/data/connection-types/{id}`                        | Delete a connection type                       |
| POST   | `/api/v1alpha1/data/test/credentials`                             | Test credentials without persisting            |
| POST   | `/api/v1alpha1/audit/data-connection-types`                       | Audit all connection types via flight service  |

## Container Images

Each service has its own `Containerfile` with multi-stage builds and dependency caching.

```sh
# Build individual images
make container-flight
make container-rest

# Build all
make container-all

# Run
make container-run-flight
make container-run-rest
```

## Make Targets

Run `make help` for the full list. Key targets:

| Target                | Description                                        |
| --------------------- | -------------------------------------------------- |
| `all`                 | Build + fmt + lint + test + audit                  |
| `build`               | `cargo build --workspace`                          |
| `release`             | `cargo build --workspace --release`                |
| `check`               | `cargo check --workspace`                          |
| `clean`               | `cargo clean`                                      |
| `test`                | Run all tests                                      |
| `test-unit`           | Unit tests (commons, connectors, kube-utils, pg-meta-store, rest) |
| `test-integration`    | Integration tests (flight-service)                 |
| `lint`                | Clippy + rustfmt check                             |
| `fmt`                 | Format all crates                                  |
| `doc`                 | Rustdoc with `-D warnings`                         |
| `audit`               | `cargo audit`                                      |
| `check-dco`           | Verify DCO sign-off on commits                     |
| `container-flight`    | Build flight-service container image               |
| `container-rest`      | Build rest-service container image                 |
| `container-all`       | Build all container images                         |
| `container-run-flight`| Run flight-service container (host network)        |
| `container-run-rest`  | Run rest-service container (host network)          |
| `generate-openapi-docs` | Bundle and build OpenAPI docs (needs redocly or container) |
| `setup-hooks`         | Install git pre-commit hooks                       |
| `sdk-install`         | Install SDK in editable mode with dev deps         |
| `sdk-test`            | Run SDK unit tests with coverage                   |
| `sdk-lint`            | Lint and format-check SDK                          |
| `sdk-fmt`             | Format SDK code                                    |
| `sdk-typecheck`       | Run mypy on SDK                                    |
| `sdk-build`           | Build SDK distribution                             |
| `sdk-all`             | Lint + typecheck + test SDK                        |
| `oc-setup-flight`     | Apply OpenShift build config for flight-service    |
| `oc-setup-rest`       | Apply OpenShift build config for rest-service      |
| `oc-setup-all`        | Apply OpenShift build configs for all services     |
| `oc-build-flight`     | Start OpenShift build for flight-service           |
| `oc-build-rest`       | Start OpenShift build for rest-service             |
| `oc-build-all`        | Start OpenShift builds for all services            |

## Key Open Source Crates

| Crate | Description |
| ----- | ----------- |
| [Apache Arrow](https://crates.io/crates/arrow) | In-memory columnar format for efficient analytical data processing and zero-copy reads |
| [Arrow Flight](https://crates.io/crates/arrow-flight) | gRPC-based protocol for high-throughput transfer of Arrow columnar data |
| [Tonic](https://crates.io/crates/tonic) | Async gRPC framework built on Hyper and Tower for HTTP/2-based services |
| [Actix Web](https://crates.io/crates/actix-web) | High-performance async HTTP server and web framework |
| [SQLx](https://crates.io/crates/sqlx) | Compile-time checked async SQL toolkit with native PostgreSQL driver |
| [Tokio](https://crates.io/crates/tokio) | Async runtime providing task scheduling, I/O, and timers |
| [Clap](https://crates.io/crates/clap) | Command-line argument parser with derive-based declarative API |
| [Serde](https://crates.io/crates/serde) | Serialization/deserialization framework for JSON and other formats |
| [Tracing](https://crates.io/crates/tracing) | Structured, event-based diagnostics and instrumentation |
| [OpenDAL](https://crates.io/crates/opendal) | Unified data access layer for storage services (S3, HDFS, GCS, Azure Blob, local FS, and more) |

## License

See [SECURITY.md](SECURITY.md) for the security policy and vulnerability reporting.
