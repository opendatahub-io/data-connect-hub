# AGENTS.md

This file provides guidance to coding agents when
working with code in this repository.

## Requirements

- Rust stable 1.96+
- Go 1.23+ (for dc-controller)
- Python 3.11+ (for SDK)
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
cargo test -p sqlite-connector -- test_name
cargo test -p flight-service -- test_name
cargo test -p rest-service -- test_name
```

Python SDK:

```console
make sdk-test       # run SDK unit tests with coverage
make sdk-lint       # lint and format-check SDK
make sdk-fmt        # format SDK code
make sdk-typecheck  # run mypy on SDK
make sdk-all        # lint + typecheck + test
```

## Architecture

**Directory layout:**

```text
services/         binary crates (flight-service, rest-service)
connectors/       data source connectors (connectors/postgres, connectors/sqlite)
libs/             shared libraries (commons, pg-meta-store, kube-utils)
dc-controller/    Go-based ODH operator controller
sdk/python/       Python SDK (REST client)
config/           Kustomize deployment configs
hack/             scripts and tooling
docs/             project documentation
```

**Crate dependency flow:**

```text
services/flight (binary, gRPC :50051)
  -> libs/commons
  -> connectors/postgres -> libs/commons

services/rest (binary, HTTP :8080)
  -> libs/commons
  -> connectors/postgres -> libs/commons
```

- **libs/commons**: shared traits (`SQLReader`), types
  (`OutputStream`), and error definitions (`ConnectorError`)
- **connectors/postgres**: library that executes SQL
  queries against PostgreSQL via SQLx and streams
  results as Arrow `RecordBatch`es
- **services/flight**: Apache Arrow Flight gRPC server
  built with tonic; implements `FlightService` trait
  for columnar data transfer
- **services/rest**: HTTP API built with actix-web for
  connection metadata listing and data access

## Key Patterns

- **Streaming over buffering**: use `SQLReader::read`
  which returns a `Stream<Item = Result<RecordBatch>>`
  with configurable batch sizes. Do not collect full
  result sets into memory.
- **Arrow as the interchange format**: all tabular
  data flows through `arrow::record_batch::RecordBatch`.
  PostgreSQL types are mapped to Arrow types in
  `connectors/postgres/src/reader.rs`.
- **Trait-based data access**: data source connectors
  implement the `SQLReader<RecordBatch>` trait from
  commons. New connectors follow this pattern.

## Adding a Data Connector

1. Create a new crate under `connectors/`
2. Add it to `Cargo.toml` workspace members
3. Implement `SQLReader<RecordBatch>` from `commons::api`
4. Map source-specific types to Arrow `DataType`
5. Add unit tests for type mapping and streaming

## REST API Routes

All routes are under `/api/v1/data`:

- `GET /health` — health check
- `GET /api/v1/data/connections` — list connections
- `POST /api/v1/data/connections` — create connection
- `GET /api/v1/data/connections/{id}` — get connection
- `PATCH /api/v1/data/connections/{id}` — update connection
- `DELETE /api/v1/data/connections/{id}` — delete connection
- `GET /api/v1/data/connection-types` — list connection types
- `POST /api/v1/data/connection-types` — create connection type
- `GET /api/v1/data/connection-types/{id}` — get connection type
- `PATCH /api/v1/data/connection-types/{id}` — update connection type
- `DELETE /api/v1/data/connection-types/{id}` — delete connection type

## Container Builds

Each service has its own `Containerfile` under
`services/` with multi-stage UBI9-minimal builds
and dependency caching. Build context is the
workspace root.

```console
make container-flight   # flight-service image
make container-rest     # rest-service image
make container-all      # both service images
```

The dc-controller has its own `Containerfile.konflux`
and is built from its own Makefile:

```console
cd dc-controller && make docker-build
```

## CI (GitHub Actions)

Workflows under `.github/workflows/`:

- **`build-and-test.yml`** — runs on every PR and push
  to `main`. Build, clippy, rustfmt check, unit tests,
  rustdoc warnings, and `cargo audit`. Uses
  `Swatinem/rust-cache` for dependency caching.
- **`python-sdk.yml`** — runs on PRs touching `sdk/python/`.
  Lint, typecheck, and unit tests for the Python SDK.
- **`ci-dco-signoff.yml`** — runs on PRs. Verifies all
  commits have a `Signed-off-by:` trailer (DCO). Use
  `git commit -s` to sign off.
- **`ci-signed-commits.yml`** — runs on PRs. Verifies
  all commits have a valid GPG/SSH signature.
- **`ci-check-typos.yml`** — runs on PRs. Checks for
  common typos using `crate-ci/typos`.
- **`ci-stale-reviews.yml`** — dismisses stale PR
  reviews after new pushes.

```console
make check-dco   # run DCO check locally
```

## ODH Integration Notes

- The ODH operator module manifests currently use **Kustomize v5** (following the MLflow operator pattern).
  The ODH team is actively migrating modules to **Helm v2 charts** (FeastOperator, OGX, Kserve, odh-observability
  all use Helm). We should plan to convert our `dc-controller/config/` overlays to a Helm chart in a future release.
  Reference PRs: opendatahub-operator#3813 (OGX/Helm), #3654 (MLflow/Kustomize migration).
- The DataConnectService CRD uses API group `dataconnecthub.opendatahub.io`.
- The CR is cluster-scoped and singleton (`default-dataconnectservice`).
- Status follows the PlatformObject contract: `observedGeneration`, `distribution`, `releases`,
  and conditions `Ready`, `ProvisioningSucceeded`, `Degraded`.
- Application images are resolved from env vars for disconnected/air-gapped support:
  `RELATED_IMAGE_ODH_DATA_CONNECT_HUB_REST_IMAGE` (rest-service) and
  `RELATED_IMAGE_ODH_DATA_CONNECT_HUB_FLIGHT_IMAGE` (flight-service).
