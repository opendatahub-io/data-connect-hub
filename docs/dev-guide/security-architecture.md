# Data Connect Hub - Security Architecture

This document describes the security architecture of Data Connect Hub (DCH) for the Security Architecture Review (SAR) with Product Security ([RHOAIENG-97018](https://redhat.atlassian.net/browse/RHOAIENG-97018)). It covers authentication, network security, secret management, the RBAC model, data flows, and trust boundaries, and ends with known risks and open questions for the reviewers.

User-facing configuration of authentication and roles is documented in [auth.md](../user-guide/auth.md). The resource model comes from [ODH-ADR-0001](https://github.com/opendatahub-io/architecture-decision-records/blob/main/architecture-decision-records/data-connect-hub/ODH-ADR-0001-data-connect-hub.md).

## 1. System overview

DCH stores connection metadata for external data sources and lets users read data from those sources as Apache Arrow streams without handling the source credentials themselves.

| Component | Language | Role | Exposed ports |
|---|---|---|---|
| `dc-controller` | Go | Operator. Reconciles the cluster-scoped singleton `DataConnectService` CR, deploys REST and Flight, registers Flight services and connection types through the REST API, and migrates ODH Dashboard connection Secrets into DCH connections. | metrics (TLS) |
| `rest-service` | Rust (actix-web) | HTTP API for connection and connection-type metadata, credential tests, readiness checks, binary download, and credential export. Runs behind a `kube-rbac-proxy` sidecar. | 8443 (TLS, proxy), 9090 (metrics) |
| `flight-service` | Rust (tonic, Arrow Flight) | gRPC data plane. Resolves connection credentials and streams data from external sources as Arrow `RecordBatch`es. | 8443 (TLS), 9090 (metrics) |
| Metadata store | PostgreSQL | Stores connections, connection types, and Flight service registrations as JSONB. Holds no credentials. | 5432 |
| Connectors | Rust libraries in Flight | PostgreSQL/pgvector, S3, Elasticsearch, Milvus, Neo4j, URI (HTTP), SQLite. | outbound only |

The data plane namespace is `redhat-ods-applications` on RHOAI and `opendatahub` on ODH. Tenants are Kubernetes namespaces, selected per request with the `X-Tenant-Id` header.

```text
                         ┌─────────────────────────── cluster ───────────────────────────────────────┐
  user / SDK / Dashboard │  ┌───────────────┐        DCH namespace                                   │
  ─────── HTTPS ─────────┼─►│ RHOAI gateway │──TLS──►┌────────────────────────────┐                   │
  Bearer token           │  │ (kube-auth-   │        │ rest-service pod           │                   │
  X-Tenant-Id            │  │  proxy)       │        │  kube-rbac-proxy :8443     │                   │
                         │  └──────┬────────┘        │    │ TokenReview + SAR      │                   │
                         │         │                 │    ▼ (loopback, plaintext)  │                   │
                         │         │                 │  rest-server 127.0.0.1:8080│                   │
                         │         │                 └──────┬──────────┬──────────┘                   │
                         │         │ TLS (gRPC)             │ TLS      │ K8s API (REST SA)            │
                         │         │                        │ REST SA  ▼                              │
                         │         ▼                        ▼ token  tenant Secrets ◄── Flight SA ─┐  │
                         │  ┌────────────────────────────────────┐                                 │  │
                         │  │ flight-service :8443               │─────────────────────────────────┘  │
                         │  │  TokenReview + SAR (data-store)    │                                    │
                         │  └──────┬─────────────────┬───────────┘                                    │
                         │         │ TLS optional    │ TLS (sslmode)                                  │
                         │         │                 ▼                                                │
                         │         │          PostgreSQL metadata store ◄── rest-service              │
                         │  dc-controller ──TLS + SA token──► rest-service                            │
                         └─────────┼──────────────────────────────────────────────────────────────────┘
                                   ▼
                     external data sources (PostgreSQL, S3, Elasticsearch, Milvus, Neo4j, HTTP)
```

## 2. Trust boundaries

| # | Boundary | What crosses it | Control |
|---|---|---|---|
| TB1 | Client to cluster edge | User bearer token, tenant header, request payloads (including inline credentials) | RHOAI gateway TLS termination and `kube-auth-proxy`; DCH re-validates every token itself |
| TB2 | Gateway to DCH pods | Same as TB1 | Re-encrypted TLS to service-CA certificates; NetworkPolicy admits only gateway pods on 8443 |
| TB3 | `kube-rbac-proxy` to `rest-server` | Authorized request plus `x-remote-user` identity header | Loopback only (`127.0.0.1:8080`); the proxy strips client-supplied `x-remote-*` headers |
| TB4 | REST to Flight | REST ServiceAccount token, tenant header, inline test credentials | TLS with service CA; Flight TokenReview + SAR; NetworkPolicy |
| TB5 | DCH services to Kubernetes API | TokenReview, SubjectAccessReview, Secret reads/writes | ServiceAccount RBAC (section 4.2) |
| TB6 | DCH services to metadata store | Connection metadata, Secret names | Database credentials from a mounted Secret; TLS with `verify-ca`/`verify-full` supported |
| TB7 | Flight to external data sources | Source credentials, queries, returned data | Per-connection TLS and CA settings; no destination restrictions (see 8.3) |
| TB8 | Controller to REST | Controller ServiceAccount token, connection types, migrated connections | TLS with service CA; `kube-rbac-proxy` SAR; NetworkPolicy |

## 3. Authentication

### 3.1 External callers

- Callers send `Authorization: Bearer <token>`. Any token the Kubernetes API server accepts through TokenReview works: OpenShift OAuth tokens, ServiceAccount tokens, or OIDC tokens when the cluster uses an external OIDC provider.
- DCH does not implement OIDC flows or issue tokens itself, and has no API keys. Interactive OAuth/OIDC login is handled by the RHOAI gateway's `kube-auth-proxy` before requests reach DCH.
- REST: `kube-rbac-proxy` performs TokenReview, then SubjectAccessReview, strips the `Authorization` header, and forwards the username as `x-remote-user`. The REST server trusts this header only because it listens on loopback and the proxy removes client-sent copies.
- Flight: an in-process tower middleware performs TokenReview and SubjectAccessReview on every call except gRPC health checks (`services/flight/src/flight/auth.rs`, `libs/kube-utils/src/auth.rs`).
- Token audiences are configurable with `spec.tokenReviewAudiences` on the `DataConnectService` CR (required on ROSA, where it must be the cluster OIDC issuer). Flight also checks that TokenReview returned a compatible audience.
- Flight caches TokenReview and SAR results in memory: 60 seconds in the shipped config (`[auth] cache_ttl_secs`), 300 seconds if unset. Token revocation and RBAC changes take up to that long to apply on Flight. REST (`kube-rbac-proxy`) is not cached.

### 3.2 Service-to-service

| Caller | Callee | Credential | Callee verifies |
|---|---|---|---|
| REST | Flight | REST ServiceAccount token (read from the mounted token file) | TokenReview + SAR `get data-store` in the tenant namespace |
| REST | Flight (connector discovery `DoAction`) | REST ServiceAccount token | TokenReview; the tenant header is optional only for this action and only for the configured discovery ServiceAccount |
| Controller | REST | Controller ServiceAccount token | `kube-rbac-proxy` TokenReview + SAR |
| REST, Flight, controller | Kubernetes API | Own ServiceAccount token | Kubernetes RBAC |
| REST, Flight | PostgreSQL | Username/password from Secret `dch-database-config` | PostgreSQL auth |

There is no mTLS. Service identity is the ServiceAccount bearer token, validated by TokenReview, combined with server-authenticated TLS and NetworkPolicy.

## 4. Authorization and RBAC

### 4.1 User-facing model

DCH defines virtual resources in API group `dataconnecthub.opendatahub.io` that exist only for SubjectAccessReview: `data-connections`, `data-connection-types`, `data-store`, and `flight-services`. Every check is namespaced to the tenant from `X-Tenant-Id`, so access to one tenant never implies access to another.

| Operation | Resource and verb checked |
|---|---|
| Connection and connection-type CRUD | `data-connections` / `data-connection-types` with `get`, `create`, `patch`, `delete` |
| Readiness check, credential test | `create` on `data-connections` |
| Binary download, all tenant-scoped Flight calls | `get` on `data-store` |
| Credential export | `create` on core `secrets` (proxy) plus `get` on the source Secret (REST, by SAR on the username) |
| Flight registration (internal) | `flight-services` with `create`, `patch`, `delete` |

Predefined ClusterRoles, bound per tenant with a RoleBinding: `dch-read`, `dch-read-write`, `dch-admin` (same as read-write today), and `dch-secret-export`. The REST mapping lives in the `kube-rbac-proxy` config file (`config/base/rest-service/kube-rbac-proxy-configmap.yaml`); the full endpoint table is in [auth.md](../user-guide/auth.md#51-custom-roles).

Reading data never requires access to Secrets. DCH resolves credentials with its own ServiceAccounts and never returns them, so a `dch-read` user can query a source without learning its credentials. Export is the only path that reveals credentials, and it requires the caller to already be able to read the source Secret.

### 4.2 ServiceAccount permissions

| ServiceAccount | Permission | Scope |
|---|---|---|
| `flight-service-sa` | `system:auth-delegator` (TokenReview, SAR) | Cluster |
| `flight-service-sa` | `get` on named connection Secrets | Per tenant, Role with `resourceNames`, created by the tenant admin |
| `rest-service-sa` | TokenReview, SAR (for `kube-rbac-proxy` and export checks) | Cluster |
| `rest-service-sa` | `get` on `data-store` | Cluster (lets REST call Flight for any tenant) |
| `rest-service-sa` | `get`, `create`, `patch`, `delete` on Secrets | Per tenant, RoleBinding to `dch-rest-service-secret-access`, created by the tenant admin |
| `dc-controller` | CRUD on configmaps, secrets, serviceaccounts, services, deployments, networkpolicies, httproutes, servicemonitors, clusterroles, clusterrolebindings; DCH virtual resources; TokenReview, SAR | Cluster |

Tenant admins opt each namespace in by creating the Flight and REST bindings; DCH does not grant itself Secret access in tenant namespaces.

### 4.3 Tenant isolation in the metadata store

All tenants share one PostgreSQL database. Isolation is enforced in application queries by filtering on `metadata.tenant_id`; there is no row-level security. Connection types with the global tenant ID (`opendatahub` by default) are readable from every tenant.

## 5. Network security

### 5.1 Transport encryption

| Hop | Protocol | Certificate source | Notes |
|---|---|---|---|
| Client to gateway | HTTPS / gRPC over TLS | RHOAI gateway | Managed by RHOAI |
| Gateway to REST, Flight | TLS | OpenShift service CA (`service.beta.openshift.io/serving-cert-secret-name`) | RHOAI `DestinationRule` re-encrypts |
| `kube-rbac-proxy` to `rest-server` | Plain HTTP | n/a | Loopback inside the pod only |
| REST to Flight | TLS (gRPC) | Service CA bundle in a ConfigMap | Server authentication only |
| Controller to REST | TLS | Service CA bundle, reloaded on change | Server authentication only |
| REST, Flight to PostgreSQL | TLS via `sslmode` | CA auto-detected at `/secrets/postgresql-ca.crt` | `verify-ca`/`verify-full` supported; mode set by the database URL |
| Flight to external sources | Connector-specific TLS | Optional per-connection `CA_CERT` | TLS use depends on the source URL and settings |
| Services to OTLP collector | gRPC TLS | `spec.trace.certificate` | `spec.trace.insecure` disables TLS |

TLS implementation: rustls with the aws-lc-rs crypto provider in both Rust services; Go `crypto/tls` in the controller. The controller reads the cluster TLS security profile (`apiservers.config.openshift.io`) and applies its minimum version and cipher suites to Flight and to `kube-rbac-proxy`. A profile whose cipher list is entirely unusable fails reconciliation, and Flight refuses to start on a minimum version its rustls build cannot serve (for example TLS 1.0) instead of downgrading.

### 5.2 NetworkPolicy

The controller deploys ingress NetworkPolicies:

- REST 8443: from gateway pods and the controller only. The plaintext REST port is not exposed to the pod network.
- Flight 8443: from gateway pods and REST pods only.
- 9090 (metrics) on both: from any source.

No egress policies are defined. Flight needs egress to arbitrary data sources by design.

## 6. Secret management

### 6.1 Where credentials live

- Data source credentials are stored only as Kubernetes Secrets in the tenant namespace. The metadata store keeps the Secret name (`credentials_ref`) and non-secret connection properties.
- Connection types mark credential fields as `sensitive` so the UI can mask them.
- Database credentials for the metadata store come from the `dch-database-config` Secret, mounted into REST and Flight.
- Service CA keys and certificates are managed by OpenShift and mounted from Secrets.

### 6.2 Credential flows

| Flow | Path | Credential handling |
|---|---|---|
| Create connection with inline credentials | Client → REST | REST validates the fields against the connection type schema, creates a Secret in the tenant namespace with its own ServiceAccount, then stores metadata. Rolls back the Secret if the metadata write fails. |
| Create connection with `credentials_ref` | Client → REST | Stores only the Secret name. The Secret is not read at creation time. |
| Query data | Client → Flight, or REST → Flight | Flight reads the Secret with its own ServiceAccount, keeps the connection pool in a short-lived in-memory cache, and never returns credential values. |
| Readiness check | Client → REST → Flight | REST reads the Secret to validate it against the schema; Flight then tests the connection. Only a status message is stored. |
| Test credentials | Client → REST → Flight | Credentials travel in the request body over TLS and are not persisted. |
| Export | Client → REST | REST checks by SAR that the caller can `get` the source Secret, then copies credentials into a new or existing Secret in the same tenant namespace. |
| Dashboard migration | Controller | Watches Secrets labeled `opendatahub.io/dashboard` with the `opendatahub.io/connection-type-ref` annotation and registers matching DCH connections by Secret reference. |

### 6.3 Leakage controls

- Config logging redacts database URL passwords and keys named like `password`, `secret`, `token`, `api_key`, or `private_key`.
- API responses never include Secret values except through export.
- Containers use `readOnlyRootFilesystem`; credentials are not written to disk, except the Neo4j connector, which writes a provided CA certificate (public data) to a temporary file.

## 7. Data flows

### 7.1 Direct Flight query

1. Client sends a Flight call with its bearer token and `X-Tenant-Id` through the gateway.
2. Flight performs TokenReview, then SAR for `get data-store` in the tenant namespace.
3. Flight loads the connection from the metadata store, scoped to the tenant.
4. Flight reads the referenced Secret with `flight-service-sa`.
5. The connector connects to the external source and streams Arrow `RecordBatch`es back to the client. Results are streamed, not buffered.

### 7.2 REST-mediated access (binary download, readiness, credential test)

1. Client sends the HTTP request through the gateway to `kube-rbac-proxy`.
2. `kube-rbac-proxy` authenticates the caller and authorizes the mapped resource and verb in the tenant namespace.
3. REST calls Flight with its own ServiceAccount token and the caller's tenant header. REST holds cluster-wide `get data-store`, so Flight trusts REST to have authorized the end user already.
4. Flight resolves credentials and talks to the source as in 7.1.

### 7.3 Controller provisioning

1. The controller reconciles the `DataConnectService` CR and applies Deployments, Services, ConfigMaps, RBAC, NetworkPolicies, and the HTTPRoute.
2. It registers each Flight service and the out-of-box connection types (`InitDataConnectionType`) and connections (`InitDataConnection`) through the REST API with its ServiceAccount token.

## 8. Known risks and open questions

These are the items we want ProdSec to weigh in on. Items marked (confirm) still need verification by the DCH team.

1. **Secret references are not authorized at creation.** A user with `create` or `patch` on `data-connections` can point `credentials_ref` at any Secret in the tenant namespace. Readiness and queries then use the REST and Flight ServiceAccounts to read it. Values are never returned, and Flight can only read Secrets the tenant admin named in its Role, but DCH may connect to a source using credentials the user could not read. Is this acceptable for Tech Preview, or should create and patch also check `get` on the Secret?
2. **REST has cluster-wide `get data-store`.** REST acts as a trusted intermediary for Flight. A compromised REST pod could read data in any tenant whose Secrets Flight can read.
3. **Outbound connections are unrestricted (SSRF).** Users with `dch-read-write` choose hosts and URLs, and the credential-test endpoint lets them make Flight connect anywhere it can reach, including in-cluster services. There is no destination allowlist or egress NetworkPolicy. Connector connection errors are returned to the caller verbatim (gRPC `UNAVAILABLE`, REST `502 connection_check_failed`); for the URI connector a failed request's error includes the upstream HTTP response body, so responses from internal endpoints can be reflected back to the caller.
4. **SQLite connector is compiled in and enabled by default.** No SQLite connection type ships, but tenant users can create connection types. A SQLite URI refers to a path inside the Flight pod. Should it be disabled in product builds?
5. **Authentication cache delays revocation** on Flight by up to the cache TTL (60 seconds as shipped).
6. **Token cache key** is a 64-bit non-cryptographic hash (`DefaultHasher`) of the bearer token. A collision would map one token to another caller's cached identity. Unlikely, but a cryptographic hash would remove the concern.
7. **No mTLS** between services; identity relies on ServiceAccount tokens plus NetworkPolicy.
8. **Metrics endpoints (9090) are unauthenticated** and open to any pod by NetworkPolicy (confirm metrics contain no tenant-sensitive labels).
9. **Controller privilege.** The controller holds cluster-wide CRUD on Secrets, ClusterRoles, and ClusterRoleBindings, making it the most privileged DCH component.
10. **Shared metadata database** with application-level tenant filtering and one database credential shared by REST and Flight.
11. **Optional TLS** to external sources and to the OTLP collector is left to configuration.
12. **Export authorizes by username only.** Group membership cannot be passed reliably through `kube-rbac-proxy`, so users authorized only through group bindings are denied (fails closed).

## 9. Hardening and supply chain

- Pods run as non-root with `allowPrivilegeEscalation: false`, `readOnlyRootFilesystem: true`, and the `RuntimeDefault` seccomp profile.
- Images are multi-stage UBI9-minimal builds; application and `kube-rbac-proxy` images are resolved from `RELATED_IMAGE_*` env vars for disconnected installs.
- CI runs `cargo audit` and `cargo deny` (advisories, licenses, bans, sources) on every PR, with stale ignores failing the build. Snyk scanning is onboarded ([RHOAIENG-97044](https://redhat.atlassian.net/browse/RHOAIENG-97044)).
- FIPS status is tracked in [RHOAIENG-97019](https://redhat.atlassian.net/browse/RHOAIENG-97019); SHA-1 use was removed in [RHOAIENG-97016](https://redhat.atlassian.net/browse/RHOAIENG-97016).
- Container manifesting and the CVE SLA process are tracked in [RHOAIENG-97042](https://redhat.atlassian.net/browse/RHOAIENG-97042) and [RHOAIENG-97043](https://redhat.atlassian.net/browse/RHOAIENG-97043).

## 10. Audit and observability

- REST and Flight can write an audit record for every audited API operation (who, what, success or failure). Audit logging is off by default and is enabled with `spec.audit` on the `DataConnectService` CR. See [audit.md](../user-guide/audit.md).
- OpenTelemetry tracing and Prometheus metrics are optional; see [tracing.md](../user-guide/tracing.md) and [metrics.md](../user-guide/metrics.md).

## 11. References

- [Authentication and authorization guide](../user-guide/auth.md)
- [ODH-ADR-0001: Data Connect Hub](https://github.com/opendatahub-io/architecture-decision-records/blob/main/architecture-decision-records/data-connect-hub/ODH-ADR-0001-data-connect-hub.md)
- [RHOAIENG-79047: Data Connect Hub security requirements](https://redhat.atlassian.net/browse/RHOAIENG-79047)
- Deployment manifests: `config/base/`, controller RBAC: `dc-controller/config/rbac/`
