# Data Connect Hub - Concepts and Terminologies

| Term | Definition |
|---|---|
| Tenant | **A Kubernetes namespace.**<br>Clients select it with the `X-Tenant-Id: <tenant-namespace>` header, and DCH checks the caller's Kubernetes permissions in that namespace. See [Authentication & Authorization](auth.md). |
| DCH user | **A user granted a DCH ClusterRole (`dch-read`, `dch-read-write`, or `dch-admin`) in a tenant.**<br>See [Authentication & Authorization](auth.md). |
| DCH namespace | **The namespace containing the DCH application CR, REST/Flight services, and built-in connection types.**<br>Platform-managed installations default to `odh-dch-infra` on ODH or `redhat-dch-infra` on RHOAI, with an optional namespace override. The standalone guide uses `opendatahub`. Its connection types, including the built-in ones, are visible to every DCH user, whichever tenant they select. |
| Global tenant | **A tenant namespace that a platform admin designates as global.**<br>See [Global Tenant](global-tenant.md). |
| DCH admin | **A user granted the `dch-admin` ClusterRole in a tenant.**<br>Manages connection types and connections there, typically as a member of a DCH admin group. |
| DCH admin group | **A group granted `dch-admin` in a tenant, so its members manage connection types and connections there.**<br>See [Setting up a DCH admin group](#setting-up-a-dch-admin-group). |

## Cluster-scoped connection types

The DCH namespace holds the cluster-scoped connection types, including the built-in ones. Every DCH user can access them, whichever tenant they select.

dc-controller keeps the DCH namespace in the `[global-connection-types]` setting of both services and never adds a global tenant. To view the setting in the standalone guide's `opendatahub` namespace (substitute your DCH namespace for a platform-managed or customized installation):

```bash
oc get configmap dch-flight-service-config -n opendatahub -o jsonpath='{.data.config\.toml}'
oc get configmap dch-rest-service-config -n opendatahub -o jsonpath='{.data.config\.toml}'
```

```toml
[global-connection-types]
tenant-ids = ["opendatahub"]
```

Manual edits to `tenant-ids` are overwritten on the next reconcile.

## Setting up a DCH admin group

Create a dedicated DCH admin group to manage connection types and connections in a tenant.

```bash
export TENANT_NS=<tenant-namespace>  # any tenant, including a global tenant such as global-assets
export ADMIN_GROUP=dch-admins

oc adm groups new "$ADMIN_GROUP"
oc adm groups add-users "$ADMIN_GROUP" alice bob
oc get group "$ADMIN_GROUP" -o yaml
```

```bash
oc adm policy add-role-to-group dch-admin "$ADMIN_GROUP" \
  -n "$TENANT_NS" \
  --rolebinding-name=dch-admins

oc get rolebinding dch-admins -n "$TENANT_NS" -o yaml
```

- Use the users' actual OpenShift usernames. If an identity provider owns the group, manage membership there and bind the resulting group name.
