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

### REST service secret permissions

The REST service also accesses Kubernetes secrets using its own ServiceAccount, independently of the caller's API permissions. The controller installs the predefined `dch-rest-service-secret-access` ClusterRole with `get`, `create`, `patch`, and `delete` on secrets. An admin must bind it to the REST ServiceAccount in each tenant namespace using a RoleBinding. The ClusterRole alone grants no access; DCH does not create a ClusterRoleBinding for it.

| REST operation | Secret permissions required by the REST ServiceAccount |
|---|---|
| Create a connection with `credentials_ref` | None during creation; this only stores the reference |
| Create a connection with inline `credentials` | `create`; `delete` to remove the new secret if storing connection metadata fails |
| Check connection readiness | `get` on the referenced secret |
| Export a connection to a secret | `get` on the source; `patch` on the destination, plus `create` when server-side apply creates it |
| Test credentials without saving them | None |

Deleting a connection removes its metadata, not its credential secret. The `delete` permission above is for failed-creation cleanup.

Save the following as `rest-secret-rbac.yaml`, replace the namespace and ServiceAccount values, and apply it with `kubectl apply -f rest-secret-rbac.yaml`:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: dch-rest-service-secret-access
  namespace: team-alpha
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: dch-rest-service-secret-access
subjects:
  - kind: ServiceAccount
    name: dch-rest-service-sa
    namespace: redhat-ods-applications
```

The RoleBinding's `metadata.namespace` must match the tenant (`X-Tenant-Id`); the subject namespace is where REST runs. For example, REST running in `redhat-ods-applications` needs this RoleBinding in `team-alpha` to create credentials for that tenant. It only needs these secret permissions in `redhat-ods-applications` if that namespace is also used as a tenant.

This example supports dynamic secret names throughout one tenant namespace. For read-only access to existing secrets, create a tenant-local Role granting `get` with `resourceNames` and bind it instead. Kubernetes cannot restrict the `create` verb using `resourceNames`. Flight still needs its separate secret-read grant from step 4, using the actual Flight deployment's ServiceAccount and allowed secret names.

Caller authorization is separate: connection creation requires `create` on `data-connections.dataconnecthub.opendatahub.io`; secret export requires `create` on core `secrets` in the tenant namespace. Granting those permissions to a caller does not grant them to the REST ServiceAccount.

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

## 9. Troubleshooting

### Connection creation or export fails with `cannot_create_secret`

The REST API returns HTTP 400 with `cannot_create_secret` when the Kubernetes secret write fails. Check the REST logs for the underlying cause, such as Kubernetes `403 Forbidden` or an existing secret name.

Run these checks as an administrator with permission to impersonate the REST ServiceAccount. Replace the deployment and namespace values for your installation:

```bash
DCH_NAMESPACE=redhat-ods-applications
DCH_REST_DEPLOYMENT=dch-rest-service
TENANT_NAMESPACE=team-alpha
DCH_REST_SA=$(kubectl get deployment "$DCH_REST_DEPLOYMENT" \
  -n "$DCH_NAMESPACE" -o jsonpath='{.spec.template.spec.serviceAccountName}')

kubectl get clusterrole dch-rest-service-secret-access

kubectl logs -n "$DCH_NAMESPACE" "deployment/$DCH_REST_DEPLOYMENT" \
  -c rest-service --since=30m --tail=100

for verb in get create patch delete; do
  kubectl auth can-i "$verb" secrets -n "$TENANT_NAMESPACE" \
    --as="system:serviceaccount:${DCH_NAMESPACE}:${DCH_REST_SA}"
done
```

For the full inline-credential, readiness, and export workflow, each permission check should return `yes`. If the ClusterRole is missing, upgrade the controller to a version that installs it. If a required permission returns `no`, apply the tenant-local RoleBinding from [REST service secret permissions](#rest-service-secret-permissions), then retry. Check the ServiceAccount named in the log error: REST, Flight, and the controller use different identities.

An HTTP 403 from kube-rbac-proxy instead points to caller authorization. Check the caller's tenant permissions separately; changing the REST ServiceAccount's RoleBinding does not authorize the caller.

### Connection creation succeeds, but readiness reports `Secret cannot be read`

Creating a connection with `credentials_ref` stores metadata without reading or creating the secret. Readiness requires REST to read it, and Flight needs read access to use the connection. Verify that the secret exists in the tenant namespace and both ServiceAccounts have `get` on that secret. A successful credential test also does not prove secret access: it passes credentials directly without storing them.

### E2E tests pass, but another tenant cannot create connections

`e2e/run-e2e.sh` explicitly binds the predefined `dch-rest-service-secret-access` ClusterRole to REST in `DCH_TENANT_ID` before running tests, including when `DCH_AUTH_TOKEN` is supplied. This RoleBinding does not apply to other namespaces. Provision each new tenant's service RBAC as well as its users' API permissions.
