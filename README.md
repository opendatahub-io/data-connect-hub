# Data Connect Hub

Data Connect Hub (DCH) is a middleware service that provides a single integration point for ingesting data from multiple heterogeneous data sources. Rather than each consuming service maintaining its own client library stacks for S3, HDFS, NFS, relational databases, and others, DCH centralises connection metadata management and exposes data through two purpose-built APIs.

## Workspace Layout

```
data-connect-hub/
├── services/
│   ├── flight/                Arrow Flight gRPC service (binary)
│   └── rest/                  HTTP REST service (binary)
├── connectors/
│   ├── postgres/              PostgreSQL data reader (library)
│   └── sqlite/                SQLite data reader (library)
├── libs/
│   ├── commons/               Shared types and traits
│   ├── pg-meta-store/         PostgreSQL metadata store
│   └── kube-utils/            Kubernetes utility helpers
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
- PostgreSQL (for integration testing, for `make container-run-flight`). The default URL in `services/flight/samples/config.toml` is `"postgresql://dch_user:dch_password@localhost:5432/dch_db"`, so you need to create a user `dch_user`, with `dch_password` as password, and `dch_db` database.
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

```sh
# REST service (default: 127.0.0.1:8080)
cargo run -p rest-service -- --config {your local path}/config.toml

# Flight service (default: 127.0.0.1:50051)
cargo run -p flight-service -- --config {your local path}/config.toml
```

## REST API

| Method | Path                                | Description                  |
| ------ | ----------------------------------- | ---------------------------- |
| GET    | `/api/v1/data/connections`          | List all connections         |
| POST   | `/api/v1/data/connections`          | Create a connection          |
| GET    | `/api/v1/data/connections/{id}`     | Get a connection             |
| PATCH  | `/api/v1/data/connections/{id}`     | Update a connection          |
| DELETE | `/api/v1/data/connections/{id}`     | Delete a connection          |
| GET    | `/api/v1/data/connection-types`     | List all connection types    |
| POST   | `/api/v1/data/connection-types`     | Create a connection type     |
| GET    | `/api/v1/data/connection-types/{id}`| Get a connection type        |
| PATCH  | `/api/v1/data/connection-types/{id}`| Update a connection type     |
| DELETE | `/api/v1/data/connection-types/{id}`| Delete a connection type     |

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
| `test-unit`           | Unit tests (commons, postgres-connector, pg-meta-store, rest) |
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
| `setup-hooks`         | Install git pre-commit hooks                       |
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
