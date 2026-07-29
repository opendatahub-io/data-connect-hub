# CLAUDE.md

This file provides guidance to Claude Code
(claude.ai/code) when working with code in this
repository.

## Requirements

- Rust stable 1.96+
- PostgreSQL (for integration testing)
- Docker or Podman (for container builds)

## Quick Reference

```console
make build          # workspace build
make test           # all tests
make fmt            # format all crates
make lint           # clippy + fmt check
make doc            # rustdoc with -D warnings
make audit          # cargo audit
make container-all  # build all container images
```

Run a single test:

```console
cargo test -p commons -- test_name
cargo test -p postgres-connector -- test_name
cargo test -p flight-service -- test_name
cargo test -p rest-service -- test_name
```

## Architecture

**Crate dependency flow:**

```text
flight-service (binary, gRPC :50051)
  -> commons
  -> postgres-connector -> commons

rest-service (binary, HTTP :8080)
  -> commons
  -> postgres-connector -> commons
```

- **commons**: shared traits (`SQLReader`), types
  (`OutputStream`), and error definitions (`ApiError`)
- **postgres-connector**: library that executes SQL
  queries against PostgreSQL via SQLx and streams
  results as Arrow `RecordBatch`es
- **flight-service**: Apache Arrow Flight gRPC server
  built with tonic; implements `FlightService` trait
  for columnar data transfer
- **rest-service**: HTTP API built with actix-web for
  connection metadata listing and data access

## Key Patterns

- **Streaming over buffering**: use `SQLReader::read`
  which returns a `Stream<Item = Result<RecordBatch>>`
  with configurable batch sizes. Do not collect full
  result sets into memory.
- **Arrow as the interchange format**: all tabular
  data flows through `arrow::record_batch::RecordBatch`.
  PostgreSQL types are mapped to Arrow types in
  `postgres-connector/src/reader.rs`.
- **Trait-based data access**: data source connectors
  implement the `SQLReader<RecordBatch>` trait from
  commons. New connectors follow this pattern.

## Adding a Data Connector

1. Create a new crate under the workspace root
2. Add it to `Cargo.toml` workspace members
3. Implement `SQLReader<RecordBatch>` from `commons::api`
4. Map source-specific types to Arrow `DataType`
5. Add unit tests for type mapping and streaming

## REST API Routes

All routes are under `/v1/data`:

- `GET /v1/data/connections` — list all connections
- `GET /v1/data/connections/{namespace}` — list by
  namespace
- `GET /v1/data/connections/{namespace}/{name}` — get
  a specific connection

## Container Builds

Each service has its own `Containerfile` with
multi-stage Alpine builds and dependency caching.
Build context is the workspace root.

```console
make container-flight   # flight-service image
make container-rest     # rest-service image
make container-all      # both
```

## CI (GitHub Actions)

Four workflows under `.github/workflows/`:

- **`build-and-test.yml`** — runs on every PR and push
  to `main`. Build, clippy, rustfmt check, unit tests,
  rustdoc warnings, and `cargo audit`. Uses
  `Swatinem/rust-cache` for dependency caching.
- **`container-publish.yml`** — runs on push to `main`
  and version tags (`v*.*.*`). Builds multi-arch
  container images and pushes to `ghcr.io`. Images are
  tagged with the short commit SHA and either `latest`
  (main) or the version tag.
- **`ci-dco-signoff.yml`** — runs on PRs. Verifies all
  commits have a `Signed-off-by:` trailer (DCO). Use
  `git commit -s` to sign off.
- **`ci-signed-commits.yml`** — runs on PRs. Verifies
  all commits have a valid GPG/SSH signature.

```console
make check-dco   # run DCO check locally
```
