# Data Connect Hub - Authentication & Authorization

## 1. Overview

The standard DCH deployment requires a bearer token for API access. Include `Authorization: Bearer <token>` and `X-Tenant-Id: <namespace>` in requests. Access depends on the permissions granted in that tenant namespace.

## 2. Configure tenant access

DCH uses separate permissions for its services and for the people or applications calling its APIs. Configure service access to connection secrets for each tenant, then grant callers the DCH API access they need.

### 2.1 Create a tenant namespace

Each tenant maps to a Kubernetes namespace. Clients pass this namespace in the `X-Tenant-Id` header. Create it if it does not already exist:

```bash
kubectl create namespace team-alpha
```

### 2.2 Grant Flight Service permission to read connection secrets

Grant Flight `get` access only to the connection Secrets it needs. For each tenant with connection Secrets, create a Role and RoleBinding; DCH does not create them by default. Use one `--resource-name` per Secret.

For example:

```bash
TENANT_NAMESPACE=team-alpha
DCH_SERVICE_NAMESPACE=redhat-ods-applications
DCH_FLIGHT_SA=dch-flight-service-sa

kubectl create role dch-flight-secret-read \
  -n "$TENANT_NAMESPACE" \
  --verb=get --resource=secrets \
  --resource-name="<connection-secret-1>" \
  --resource-name="<connection-secret-2>" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create rolebinding dch-flight-secret-read-rb \
  -n "$TENANT_NAMESPACE" \
  --role=dch-flight-secret-read \
  --serviceaccount="${DCH_SERVICE_NAMESPACE}:${DCH_FLIGHT_SA}" \
  --dry-run=client -o yaml | kubectl apply -f -
```

### 2.3 Grant REST Service permission to manage connection secrets

DCH installs the `dch-rest-service-secret-access` ClusterRole. Bind it in each tenant namespace:

```bash
TENANT_NAMESPACE=team-alpha
DCH_SERVICE_NAMESPACE=redhat-ods-applications
DCH_REST_SA=dch-rest-service-sa

kubectl create rolebinding dch-rest-service-secret-access \
  -n "$TENANT_NAMESPACE" \
  --clusterrole=dch-rest-service-secret-access \
  --serviceaccount="${DCH_SERVICE_NAMESPACE}:${DCH_REST_SA}" \
  --dry-run=client -o yaml | kubectl apply -f -
```

### 2.4 Grant users access to Flight and REST APIs

DCH provides these ClusterRoles:

| Predefined ClusterRole | Access |
|---|---|
| `dch-read` | Read connections, connection types, and data-store resources. |
| `dch-read-write` | Read and modify those DCH resources. |
| `dch-admin` | Same permissions as `dch-read-write`. Reserved for future administrative operations. |
| `dch-secret-export` | Create or overwrite Secrets in a tenant namespace (required for the export endpoint). |

Minimum role by operation:
| Operation | Minimum role |
|---|---|
| List or get connections, connection types, binary data | `dch-read` |
| Create, update, or delete connections and connection types | `dch-read-write` |
| Readiness check, credential test | `dch-read-write` |
| Export connection secret | `dch-read-write` + `dch-secret-export` |

Create one RoleBinding per user and tenant. Example: grant Alice read access in `team-alpha`:

```bash
TENANT_NAMESPACE=team-alpha
DCH_USER=alice
DCH_ROLE=dch-read

kubectl create rolebinding alice-data-access \
  -n "$TENANT_NAMESPACE" \
  --clusterrole="$DCH_ROLE" \
  --user="$DCH_USER" \
  --dry-run=client -o yaml | kubectl apply -f -
```

The export endpoint (`PUT /connections/{id}/exports/secrets/{secret_name}`) requires the caller to have `create` access to Secrets in the tenant namespace. Bind the predefined `dch-secret-export` ClusterRole:

```bash
TENANT_NAMESPACE=team-alpha
DCH_USER=alice

kubectl create rolebinding alice-secret-export \
  -n "$TENANT_NAMESPACE" \
  --clusterrole=dch-secret-export \
  --user="$DCH_USER" \
  --dry-run=client -o yaml | kubectl apply -f -
```

## 3. Authentication flow

### 3.1 Flight requests

1. Send `Authorization: Bearer <token>` and `X-Tenant-Id: <namespace>`.
2. DCH validates the token and identifies the caller.
3. DCH checks the caller has `get` access to `data-connections` in the tenant namespace. `dch-read` is sufficient for all Flight operations (see 2.4).
4. For a saved connection, Flight reads its Secret using the service permission from 2.2.

#### Flight errors

| Condition | gRPC status |
|---|---|
| Missing or invalid token | `UNAUTHENTICATED` |
| Missing tenant or insufficient access | `PERMISSION_DENIED` |

#### Cache behavior

Flight caches authentication and authorization results for 300 seconds (5 minutes) by default. Revoked tokens and permission changes may take up to that long to take effect.

### 3.2 REST requests

1. Send `Authorization: Bearer <token>` and `X-Tenant-Id: <namespace>`.
2. `kube-rbac-proxy` checks the caller's tenant permissions via SubjectAccessReview (see [minimum roles](#minimum-role-by-operation) in 2.4).
3. For Secret operations, REST uses the service permissions from 2.3; these are separate from the caller's permissions.

#### REST errors

| Condition | HTTP status |
|---|---|
| Missing or invalid token | `401 Unauthorized` |
| Missing or empty `X-Tenant-Id` when required | `400 Bad Request` |
| Caller lacks permission for the requested operation or tenant | `403 Forbidden` |

## 4. Troubleshooting

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
  -c rest-server --since=30m --tail=100

for verb in get create patch delete; do
  kubectl auth can-i "$verb" secrets -n "$TENANT_NAMESPACE" \
    --as="system:serviceaccount:${DCH_NAMESPACE}:${DCH_REST_SA}"
done
```

For the full inline-credential, readiness, and export workflow, each permission check should return `yes`. If the ClusterRole is missing, upgrade the controller to a version that installs it. If a required permission returns `no`, apply the tenant-local RoleBinding from [REST service secret permissions](#23-grant-rest-service-permission-to-manage-connection-secrets), then retry. Check the ServiceAccount named in the log error: REST, Flight, and the controller use different identities.

An HTTP 403 from kube-rbac-proxy instead points to caller authorization. Check the caller's tenant permissions separately; changing the REST ServiceAccount's RoleBinding does not authorize the caller.

### Connection creation succeeds, but readiness reports `Secret cannot be read`

Creating a connection with `credentials_ref` stores metadata without reading or creating the secret. Readiness requires REST to read it, and Flight needs read access to use the connection. Verify that the secret exists in the tenant namespace and both ServiceAccounts have `get` on that secret. A successful credential test also does not prove secret access: it passes credentials directly without storing them.
