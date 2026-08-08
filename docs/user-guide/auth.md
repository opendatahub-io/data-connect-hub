# Data Connect Hub - Authentication & Authorization

## 1. Overview

Both the REST service and Flight service authenticate requests via Kubernetes TokenReview and authorize access via SubjectAccessReview (SAR).

## 2. RBAC Setup

To grant a user access to a tenant's data, an admin must:

1. **Define ClusterRoles for data access.** These describe what actions users can perform on DCH resources. The SAR check in the auth flow evaluates requests against these roles. ClusterRoles are cluster-scoped — define them once, then reference from any namespace via RoleBinding.

    Reader (query data only):

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

    Admin (full access):

    ```yaml
    apiVersion: rbac.authorization.k8s.io/v1
    kind: ClusterRole
    metadata:
      name: dch-data-connections-admin
    rules:
      - apiGroups: ["dataconnecthub.opendatahub.io"]
        resources: ["data-connections"]
        verbs: ["get", "post", "patch", "delete"]
      - apiGroups: ["dataconnecthub.opendatahub.io"]
        resources: ["data-connection-types"]
        verbs: ["get", "post", "patch", "delete"]
    ```

The available resources and verbs per command:

**REST Service**

| Endpoint | K8s Resource | K8s Verb |
|----------|----------|------|
| `GET /v1/data/connections` | data-connections | get |
| `POST /v1/data/connections` | data-connections | post |
| `GET /v1/data/connections/{id}` | data-connections | get |
| `PATCH /v1/data/connections/{id}` | data-connections | patch |
| `DELETE /v1/data/connections/{id}` | data-connections | delete |
| `GET /v1/data/connection-types` | data-connection-types | get |
| `POST /v1/data/connection-types` | data-connection-types | post |
| `GET /v1/data/connection-types/{id}` | data-connection-types | get |
| `PATCH /v1/data/connection-types/{id}` | data-connection-types | patch |
| `DELETE /v1/data/connection-types/{id}` | data-connection-types | delete |

**Flight Service**

| Operation | gRPC Method | K8s Resource | K8s Verb |
|-----------|------------|----------|------|
| Get SQL info | GetFlightInfoSqlInfo / DoGetSqlInfo | — | No authorization required |
| Query data | GetFlightInfoStatement / DoGetStatement | data-connections | get |

All resources are under the `dataconnecthub.opendatahub.io` API group, scoped to the namespace specified by `X-Tenant-Id`.

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

## 3. Auth Flow

1. Client sends a request with `Authorization: Bearer <token>` and `X-Tenant-Id: <namespace>` headers.
2. **Authentication (TokenReview)**: The bearer token is validated against the Kubernetes API server. If valid, the API server returns the user's identity (username and groups).
3. **Authorization (SubjectAccessReview)**: The system checks whether the authenticated user has the required RBAC permission in the namespace specified by `X-Tenant-Id`. The SAR checks access to the `data-connections` resource under the `dataconnecthub.opendatahub.io` API group.
4. **If authorized**: The request is forwarded to the backend service with `X-Remote-User` and `X-Remote-Groups` headers injected, carrying the authenticated identity for downstream use.
5. **If unauthorized**: The request is rejected (401 Unauthenticated or 403 Forbidden).