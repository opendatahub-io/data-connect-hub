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

### Running the Services

```sh
# REST service (default: 127.0.0.1:8080)
cargo run -p rest-service -- --port 8080

# Flight service (default: 127.0.0.1:50051)
cargo run -p flight-service
```

## REST API

| Method | Path                                       | Description              |
| ------ | ------------------------------------------ | ------------------------ |
| GET    | `/v1/data/connections`                     | List all connections     |
| GET    | `/v1/data/connections/{namespace}`          | List by namespace        |
| GET    | `/v1/data/connections/{namespace}/{name}`   | Get a specific connection|

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

| Target              | Description                                      |
| ------------------- | ------------------------------------------------ |
| `build`             | `cargo build --workspace`                        |
| `release`           | `cargo build --workspace --release`              |
| `test`              | Run all tests                                    |
| `test-unit`         | Unit tests (commons, postgres-connector, rest)   |
| `test-integration`  | Integration tests (flight-service)               |
| `lint`              | Clippy + rustfmt check                           |
| `fmt`               | Format all crates                                |
| `audit`             | `cargo audit`                                    |
| `container-all`     | Build all container images                       |

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
