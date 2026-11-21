# Data Connect Hub - Concepts and Terminologies

| Term | Definition |
|---|---|
| Tenant | **A Kubernetes namespace.**<br>Clients select it with the `X-Tenant-Id: <tenant-namespace>` header, and DCH checks the caller's Kubernetes permissions in that namespace. See [Authentication & Authorization](auth.md). |
| DCH user | **A user granted a DCH ClusterRole (`dch-read`, `dch-read-write`, or `dch-admin`) in a tenant.**<br>See [Authentication & Authorization](auth.md). |
| DCH namespace | **The namespace where DCH is installed (`opendatahub` by default), and where the installer creates the built-in connection types.**<br>Its connection types, including the built-in ones, are visible to every DCH user, whichever tenant they select. |
| Global tenant | **A tenant namespace that a platform admin designates as global.**<br>Users read its connection types and connections and ingest from its connections with `X-Tenant-Id: <global-tenant-namespace>` and `X-Data-Connection-Id: <connection-id>`. Its connection types are not visible from other tenants; share a connection type with every tenant by creating it in the DCH namespace. See [Global Tenant](global-tenant.md). |
| DCH admin | **A user granted the `dch-admin` ClusterRole in a tenant.**<br>Manages connection types and connections there, typically as a member of a DCH admin group. |
| DCH admin group | **A group granted `dch-admin` in a tenant, so its members manage connection types and connections there.**<br>See [Setting up a DCH admin group](#setting-up-a-dch-admin-group). |

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
