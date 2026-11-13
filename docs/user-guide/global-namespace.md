# Data Connect Hub - Global Namespace

Global namespaces let administrators share DCH connections with every RHOAI user through RHOAI-wide global projects. Administrators curate connection types and connections there once; users discover them and ingest data without being added to the project one by one and without seeing credentials.

## Flow

```text
┌────────────────────────────────────────┐
│ RHOAI Dashboard: Cluster Settings      │
│   Global project: [ global-assets  v ] │
│   (platform admin sets this)           │
└───────────────────┬────────────────────┘
                    │ labels the namespace
                    │ opendatahub.io/global-data-registry-project
                    ▼
┌────────────────────────────────────────┐    ┌───────────────────┐
│ dc-controller                          │    │ Auth CR "auth"    │
│ - watches the label and the Auth CR    │◄───┤ - adminGroups     │
│ - creates RoleBindings in each global  │    │ - allowedGroups   │
│   project                              │    └───────────────────┘
│ - makes connection types in global     │
│   projects visible to every tenant     │
└───────────────────┬────────────────────┘
                    │ creates RoleBindings
                    ▼
┌────────────────────────────────────────────────────┐
│ Global project (namespace: global-assets)          │
│                                                    │
│ RoleBindings:                                      │
│   adminGroups                  -> dch-read-write   │
│   allowedGroups + adminGroups  -> dch-read         │
│   DCH REST service account     -> Secret access    │
│   DCH Flight service account   -> Secret read      │
│                                                    │
│ Connection types, connections, connection Secrets  │
└───────────────────▲────────────────────────────────┘
                    │ authorizes the caller (SubjectAccessReview)
                    │ reads Secrets with its own service account
┌───────────────────┴────────────────────┐
│ DCH REST API / SDK and Flight API      │
└─────────▲─────────────────────▲────────┘
          │                     │ X-Tenant-Id: global-assets
┌─────────┴──────────┐ ┌────────┴───────────────────┐
│ adminGroups        │ │ allowedGroups              │
│ (curators)         │ │ (consumers, any project)   │
│ - create, update,  │ │ - list and get metadata    │
│   and delete       │ │ - ingest data (Flight)     │
│   connection types │ │ - no Secrets, export,      │
│   and connections  │ │   readiness or credential  │
│                    │ │   tests                    │
└────────────────────┘ └────────────────────────────┘
```

## Workflow

1. A platform admin picks the global project in Dashboard Settings, and the namespace is labeled `opendatahub.io/global-data-registry-project`. The Dashboard does not apply this label yet, see [Designate the global namespace](#designate-the-global-namespace).
2. dc-controller sees the label and creates RoleBindings in the global project, and makes its connection types visible to every tenant:
   - Members of the Auth CR `adminGroups` get read/write access (curate connection types and connections).
   - Members of the Auth CR `allowedGroups` get read access to connection types and connection metadata, and can ingest data through a connection.
   - Users in neither group can neither see nor query the global project.
   - DCH REST and Flight service accounts can store and read connection Secrets there. Users get no Secret access.
3. In the global project, members of `adminGroups` use the DCH REST API / SDK to curate connection types and connections.
4. From any project, members of `allowedGroups` call DCH with `X-Tenant-Id: <global-project>` to:
   - get connection metadata with the DCH REST API / SDK.
   - ingest data through a connection with the DCH Flight API. DCH uses the stored credentials server-side.
   - They cannot read Secrets, export a connection Secret, or run readiness / credential tests.
5. Later changes are applied automatically:
   - Auth CR group changes update the RoleBindings.
   - Labeling another namespace adds it as a global project; several global projects can coexist.
   - Unlabeling a namespace removes its RoleBindings. Its connection types are not moved, so they are no longer visible to other tenants, and connections in other tenants that reference them can no longer resolve those types.
   - The built-in connection types stay in the DCH namespace, which is always global for connection types, so they remain visible to every tenant.

Out of scope: copying Secrets into user projects.

## Designate the global namespace

TODO: the Dashboard does not apply `opendatahub.io/global-data-registry-project` yet; it only applies `opendatahub.io/global-mlflow-workspace` for the MLflow global workspace. Until it does, label the namespace yourself:

```bash
GLOBAL_NAMESPACE=global-assets
kubectl label namespace "$GLOBAL_NAMESPACE" \
  opendatahub.io/global-data-registry-project="$GLOBAL_NAMESPACE"
```

Verify the global namespaces and the RoleBindings dc-controller created in them:

```bash
kubectl get namespace -l opendatahub.io/global-data-registry-project
kubectl get rolebinding -A -l dataconnecthub.opendatahub.io/global-namespace=true
```

The `DataConnectService` status lists the global namespaces whose connection types the DCH services currently share with every tenant. The field is empty when no namespace is labeled or the platform `Auth` CRD is not installed:

```bash
kubectl get dataconnectservice -A -o wide
kubectl get dataconnectservice -A -o jsonpath='{.items[*].status.globalNamespaces}'
```

## RoleBindings

dc-controller enables global namespaces only when the platform `Auth` CRD (`services.platform.opendatahub.io`) is installed. In each labeled namespace it maintains these RoleBindings from the `Auth` CR named `auth`:

| RoleBinding | Subjects | ClusterRole |
|---|---|---|
| `dch-global-read` | `spec.allowedGroups` and `spec.adminGroups` | `dch-read` |
| `dch-global-read-write` | `spec.adminGroups` | `dch-read-write` |
| `dch-global-rest-service-secret-access` | REST ServiceAccount | `dch-rest-service-secret-access` |
| `dch-global-flight-service-secret-access` | Flight ServiceAccount | `dch-flight-service-secret-access` |

With the default `allowedGroups: [system:authenticated]`, every authenticated identity, including ServiceAccounts, gets read access. Permission changes take up to the authorization cache TTL (5 minutes by default) to take effect, as with any tenant.

dc-controller manages only the RoleBinding names above. Other RoleBindings in a global namespace are left alone, so you can still grant access as described in [Authentication & Authorization](auth.md#2-configure-tenant-access). For example, to let a group curate global connections without adding it to the platform `adminGroups`:

```bash
GLOBAL_NAMESPACE=global-assets
CURATOR_GROUP=global-data-curators

kubectl create rolebinding dch-global-curators \
  -n "$GLOBAL_NAMESPACE" \
  --clusterrole=dch-read-write \
  --group="$CURATOR_GROUP"
```

References:

- [RHAISTRAT-3058](https://redhat.atlassian.net/browse/RHAISTRAT-3058), [RHAI-14513](https://redhat.atlassian.net/browse/RHAI-14513)
- [ODH-ADR-DR-0001: Data Registry for RHOAI](https://github.com/opendatahub-io/architecture-decision-records/blob/main/architecture-decision-records/data-registry/ODH-ADR-DR-0001-data-registry.md)
- [Global Data Registry design](https://docs.google.com/document/d/16krYRDv-1d_W2n8miOZAah51a28i8-HvBgsTxTPTcjY/edit?usp=sharing)
- [ODH-ADR-ML-0002: MLflow shared workspace](https://github.com/opendatahub-io/architecture-decision-records/blob/bdcf1505f3aa9405263b40882e528814f678c98d/architecture-decision-records/mlflow/ODH-ADR-ML-0002-shared-workspace-for-cross-namespace-resource-sharing.md)
- https://redhat-internal.slack.com/archives/C0BJ2QQKB5G/p1791283162027329
- https://redhat-internal.slack.com/archives/C0BMMC93YLS/p1793744533805849
- https://redhat-internal.slack.com/archives/C07C0FNHVPT/p1793730024489649
