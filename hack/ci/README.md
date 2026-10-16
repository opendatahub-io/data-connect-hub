# CI Scripts

Scripts that provision the e2e test cluster on Kind, driven by
[`.github/workflows/e2e-test.yml`](../../.github/workflows/e2e-test.yml).
Shared variables and helpers live in `lib.sh`; every namespace below is a
`lib.sh` variable and can be overridden via environment variables.

| Script | Purpose |
|--------|---------|
| `setup-kind.sh` | Stage 1: create the Kind cluster, install Gateway API CRDs and Istio, deploy the ingress Gateway. |
| `deploy-dch.sh` | Stage 2: build images, load them into Kind, deploy DCH and the tenant datasources. |
| `run-tests.sh` | Stage 3: generate the e2e env config and run the e2e suite. |
| `run-disabled-connector-test.sh` | Verify CRUD and ingestion behavior for a disabled connector. |
| `lib.sh` | Shared variables and helpers; sourced by the other scripts. |

## Namespace layout

The test cluster spreads components across namespaces so the topology
mirrors a production install: a dedicated ingress namespace, a dedicated
operator namespace, and a workload namespace.

| Namespace | Components |
| --------- | ---------- |
| `istio-system` | istio-base, istiod |
| `dch-gateway` | Gateway, TLS secret, gateway options ConfigMap, gateway Service |
| `dch-controller` | dc-controller-manager, rest-service-ca ConfigMap |
| `dch-service` | DataConnectService CR, flight-service, rest-service, system PostgreSQL, OTEL collector, Jaeger, DestinationRules, metrics NodePorts |
| `dch-tenant` | PostgreSQL, Neo4j, Elasticsearch, Milvus, SeaweedFS (S3) |
