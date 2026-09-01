# Data Connect Hub - Authentication & Authorization

## 1. Overview

The Flight service authenticates requests via Kubernetes TokenReview and authorizes access via SubjectAccessReview (SAR). Auth is disabled by default and must be enabled in the service configuration.

The REST service is protected by kube-rbac-proxy. Its health endpoint bypasses that service-level authentication for Kubernetes probes.

## 2. Configuration

Enable auth in the Flight service `config.toml`:

```toml
[auth]
enabled = true
cache_ttl_secs = 300
token_review_audiences = ["https://kubernetes.default.svc"]
```

When `enabled = false` (the default), all requests bypass authentication.
`auth.token_review_audiences` is the list of audience identifiers that Flight explicitly trusts for TokenReview. Include only audiences intentionally trusted by Flight.
If your cluster uses a different service account token audience (for example, kind often uses `https://kubernetes.default.svc.cluster.local`), override `auth.token_review_audiences` accordingly.

## 3. Deployment Prerequisites

The Flight service's ServiceAccount must be able to call the Kubernetes TokenReview and SubjectAccessReview APIs.
The default Data Connect Hub manifests already include the required `ClusterRoleBinding`
(`dch-flight-auth-delegator`) to the built-in `system:auth-delegator` ClusterRole.

Without this, the Flight service will return internal errors on every auth attempt.

## 4. RBAC Setup

To grant a user access to a tenant's data, an admin must:

1. **Define ClusterRoles for data access.** These describe what actions users can perform on DCH resources. The SAR check in the auth flow evaluates requests against these roles. ClusterRoles are cluster-scoped — define them once, then reference from any namespace via RoleBinding.

    Reader (query data):

    ```yaml
    apiVersion: rbac.authorization.k8s.io/v1
    kind: ClusterRole
    metadata:
      name: dch-data-connections-reader
    rules:
      - apiGroups: ["dataconnecthub.opendatahub.io"]
        resources: ["data-connections"]
        verbs: ["get"]
    ```

    The Flight service currently checks the `get` verb for all operations. The `data-connections` resource under the `dataconnecthub.opendatahub.io` API group is a virtual resource used solely for authorization decisions — it does not correspond to a CRD.

2. **Create a tenant namespace.** Each tenant maps to a Kubernetes namespace — this is what clients pass as `X-Tenant-Id`.

    ```bash
    kubectl create namespace team-alpha
    ```

3. **Create a RoleBinding in the tenant namespace.** This grants a specific user (or group) the data access role within a tenant. Without this, the SAR check denies access even if the user is authenticated.

    ```yaml
    apiVersion: rbac.authorization.k8s.io/v1
    kind: RoleBinding
    metadata:
      name: alice-data-access
      namespace: team-alpha
    roleRef:
      apiGroup: rbac.authorization.k8s.io
      kind: ClusterRole
      name: dch-data-connections-reader
    subjects:
      - kind: User
        name: alice
    ```

4. **Grant Flight access to tenant secrets.** Data connection credentials are stored as Kubernetes secrets in tenant namespaces.

    - Data Connect Hub does **not** create secret-read RBAC manifests by default.
    - For each tenant namespace, an admin must create explicit RBAC for `flight-service-sa`.
    - Prefer namespace-local `Role` + `RoleBinding` with `resourceNames` for least privilege:

    ```yaml
    apiVersion: rbac.authorization.k8s.io/v1
    kind: Role
    metadata:
      name: dch-flight-secret-reader
      namespace: team-alpha
    rules:
      - apiGroups: [""]
        resources: ["secrets"]
        verbs: ["get"]
        resourceNames:
          - <allowed-connection-secret-name>
    ---
    apiVersion: rbac.authorization.k8s.io/v1
    kind: RoleBinding
    metadata:
      name: dch-flight-secret-reader
      namespace: team-alpha
    roleRef:
      apiGroup: rbac.authorization.k8s.io
      kind: Role
      name: dch-flight-secret-reader
    subjects:
      - kind: ServiceAccount
        name: flight-service-sa
        namespace: dch-services
    ```

    Replace `<allowed-connection-secret-name>` with your connection secret name and `dch-services` with the namespace where DCH services run. This cross-namespace ServiceAccount binding is valid because `RoleBinding` subjects include both `name` and `namespace`. If secret names are dynamic, create/update this tenant-local `Role` as needed.

## 5. Auth Flow

1. Client sends a request with `Authorization: Bearer <token>` and `X-Tenant-Id: <namespace>` headers.
2. **Path check**: Health check requests (`/grpc.health.v1.Health/*`) bypass Flight service authentication. All other gRPC methods require authentication.
3. **Authentication (TokenReview)**: The bearer token is validated against the Kubernetes API server. If valid, the API server returns the user's identity (username and groups).
4. **Authorization (SubjectAccessReview)**: The system checks whether the authenticated user has `get` permission on the `data-connections` resource (API group `dataconnecthub.opendatahub.io`) in the namespace specified by `X-Tenant-Id`.
5. **If authorized**: The request is forwarded to the backend with `X-Remote-User` and `X-Remote-Groups` headers injected, carrying the authenticated identity for downstream use.
6. **If unauthorized**: The request is rejected with gRPC `UNAUTHENTICATED` (missing/invalid token) or `PERMISSION_DENIED` (missing tenant header, or SAR denied).

## 6. Error Responses

| Condition | gRPC Status |
|-----------|-------------|
| Missing `Authorization` header | `UNAUTHENTICATED` — "missing bearer token" |
| Invalid or expired token | `UNAUTHENTICATED` |
| Missing or empty `X-Tenant-Id` header | `PERMISSION_DENIED` — "missing x-tenant-id" |
| User lacks RBAC in the target namespace | `PERMISSION_DENIED` — "access denied for data-connections in namespace \<ns\>" |
| Health check endpoint | No Flight service auth required |

## 7. Caching

Authentication and authorization results are cached using in-memory Moka caches (up to 10,000 entries each). The TTL is controlled by `auth.cache_ttl_secs` (default: 300 seconds). Token cache keys are SHA-256 hashes of the bearer token. Revoking a token or changing RBAC takes effect after the cache entry expires.

## 8. Known Limitations

- **Platform Gateway authentication is separate.** RHOAI and ODH platform Gateways can require a bearer token before forwarding health requests to DCH, even though DCH service-level health checks are anonymous.
- **Single verb.** The Flight service checks only the `get` verb for all operations, regardless of the gRPC method called.
- **Audience configuration must match cluster tokens.** TokenReview audiences are configurable through `auth.token_review_audiences` (default: `https://kubernetes.default.svc`), and a mismatch will cause authentication failures.
