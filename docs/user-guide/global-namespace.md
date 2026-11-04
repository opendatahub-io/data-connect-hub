# Data Connect Hub - Global Namespace

The global namespace lets administrators share DCH connections with every RHOAI user through the single RHOAI-wide global project. Administrators curate connection types and connections there once; users discover them and ingest data without being added to the project one by one and without seeing credentials.

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
│ - creates RoleBindings in the global   │    │ - allowedGroups   │
│   project                              │    └───────────────────┘
│ - makes the global project the tenant  │
│   of global connection types           │
│ - reports DataConnectService status    │
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
2. dc-controller sees the label and creates RoleBindings in the global project:
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
   - Picking a different global project moves the RoleBindings and global connection types to it.
   - Clearing the global project removes the RoleBindings and moves global connection types back to the DCH namespace.

Out of scope: multiple global projects, copying Secrets into user projects.

## Designate the global namespace

TODO: the Dashboard does not apply `opendatahub.io/global-data-registry-project` yet; it only applies `opendatahub.io/global-mlflow-workspace` for the MLflow global workspace. Until it does, label the namespace yourself:

```bash
GLOBAL_NAMESPACE=global-assets
kubectl label namespace "$GLOBAL_NAMESPACE" \
  opendatahub.io/global-data-registry-project="$GLOBAL_NAMESPACE"
```

Or set `GLOBAL_NAMESPACE_LABEL=opendatahub.io/global-mlflow-workspace` on dc-controller to follow the MLflow global workspace selected in the Dashboard.

Check the result on the DataConnectService:

```bash
kubectl get dataconnectservice -A \
  -o jsonpath='{range .items[*]}{.status.globalNamespace}{"\t"}{.status.conditions[?(@.type=="GlobalNamespaceReady")].reason}{"\n"}{end}'
```

For the RoleBinding names, controller settings, and how to add a curator group that is not in `adminGroups`, see [Global namespace](auth.md#25-global-namespace) in the authentication guide.


References:

- [RHAISTRAT-3058](https://redhat.atlassian.net/browse/RHAISTRAT-3058), [RHAI-14513](https://redhat.atlassian.net/browse/RHAI-14513)
- [ODH-ADR-DR-0001: Data Registry for RHOAI](https://github.com/opendatahub-io/architecture-decision-records/blob/main/architecture-decision-records/data-registry/ODH-ADR-DR-0001-data-registry.md)
- [Global Data Registry design](https://docs.google.com/document/d/16krYRDv-1d_W2n8miOZAah51a28i8-HvBgsTxTPTcjY/edit?usp=sharing)
- [ODH-ADR-ML-0002: MLflow shared workspace](https://github.com/opendatahub-io/architecture-decision-records/blob/bdcf1505f3aa9405263b40882e528814f678c98d/architecture-decision-records/mlflow/ODH-ADR-ML-0002-shared-workspace-for-cross-namespace-resource-sharing.md)
- https://redhat-internal.slack.com/archives/C0BJ2QQKB5G/p1791283162027329
- https://redhat-internal.slack.com/archives/C0BMMC93YLS/p1793744533805849
- https://redhat-internal.slack.com/archives/C07C0FNHVPT/p1793730024489649