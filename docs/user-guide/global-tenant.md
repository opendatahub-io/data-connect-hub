# Global Tenant

DCH connection types and connections are scoped by tenant:

- Connection types:
  - Tenant-scoped connection types: `tenant_id` is set to the tenant namespace. They are accessible only to DCH users authorized in that namespace.
  - Cluster-scoped connection types: `tenant_id` is omitted in API responses. They are accessible by every DCH user, whichever tenant they select. These are the connection types in the DCH namespace, including the built-in ones. To share a connection type with all DCH users, create it in the DCH namespace.
  - Global-tenant connection types: `tenant_id` is set to the global tenant namespace. They are accessible only to DCH users authorized in the global tenant, by selecting it with `X-Tenant-Id`: the `Auth` CR `allowedGroups` and `adminGroups`, or a group granted a DCH role there. If a user creates a connection with a global-tenant connection type and the namespace later stops being a global tenant, fetching data through that connection fails. In this case, cluster-scoped connection types should be considered.
- Connections:
  - Tenant-scoped connections: `tenant_id` is set to the tenant namespace. They are accessible only to DCH users authorized in that namespace.
  - Global-tenant connections: `tenant_id` is set to the global tenant namespace. They are accessible only to DCH users authorized in the global tenant, by selecting it with `X-Tenant-Id`.

See [Concepts and Terminologies](concepts-terminologies.md) for the terms used in this guide.

## Workflow

1. **Platform admin installs DCH.**
   The DCH installer installs the built-in connection types in the DCH namespace (`opendatahub` in this guide) as cluster-scoped connection types, so every DCH user can see them. See [Cluster-scoped connection types](concepts-terminologies.md#cluster-scoped-connection-types).

2. **Platform admin configures the `Auth` CR.**
   The `Auth` CRD is shipped with the RHOAI platform, and the platform operator creates the `auth` CR with the defaults below if it does not exist. Edit it only to change the groups. See the [API reference](https://github.com/opendatahub-io/opendatahub-operator/blob/main/docs/api-overview.md#authspec) and the [CRD type definitions](https://github.com/opendatahub-io/opendatahub-operator/blob/main/api/services/v1alpha1/auth_types.go).

   ```bash
   oc get auth auth -o yaml
   ```

   ```yaml
   apiVersion: services.platform.opendatahub.io/v1alpha1
   kind: Auth
   metadata:
     name: auth
   spec:
     adminGroups:
       - rhods-admins          # default on self-managed RHOAI; dedicated-admins on managed RHOAI, odh-admins on ODH
     allowedGroups:
       - system:authenticated  # default: every authenticated user
   ```

3. **Platform admin designates a global tenant.**
   The platform admin designates a namespace, `global-assets` in this guide, in RHOAI Dashboard Settings, which labels it `opendatahub.io/global-data-connect-hub-project`.

4. **dc-controller reconciles the global tenant.**

   This step is automatic. dc-controller:

   - Creates RoleBindings in the namespace,<br>so users access connections in `global-assets` with `X-Tenant-Id: global-assets`:

     | Subject | ClusterRole | Grants |
     |---|---|---|
     | Members of the Auth CR `adminGroups` | [`dch-read-write`](https://github.com/opendatahub-io/data-connect-hub/blob/main/manifests/charts/data-connect-hub/templates/data-clusterroles.yaml) | • read/write to connection types and connections<br>• ingest data via a connection<br>• no access to Secrets |
     | Members of the Auth CR `allowedGroups` | [`dch-read`](https://github.com/opendatahub-io/data-connect-hub/blob/main/manifests/charts/data-connect-hub/templates/data-clusterroles.yaml) | • read-only to connection types and connections<br>• ingest data via a connection<br>• no access to Secrets |
     | REST ServiceAccount | [`dch-rest-service-secret-access`](https://github.com/opendatahub-io/data-connect-hub/blob/main/manifests/charts/data-connect-hub/templates/rest-rbac.yaml) | • read/write to Secrets |
     | Flight ServiceAccount | [`dch-flight-service-secret-access`](https://github.com/opendatahub-io/data-connect-hub/blob/main/manifests/charts/data-connect-hub/templates/flight-rbac.yaml) | • read-only to Secrets |

   - Updates `DataConnectService.status`:

     ```yaml
     apiVersion: dataconnecthub.opendatahub.io/v1alpha1
     kind: DataConnectService
     metadata:
       # ...
     spec:
       # ...
     status:
       globalTenants:
         - global-assets
     ```

   Removing the label reverts both. RoleBindings that administrators create under other names are not touched. Neither step changes the service ConfigMaps, so designating or removing a global tenant does not restart the REST or Flight service. The services cache authorization decisions for `[auth] cache_ttl_secs` (60 seconds by default), so access granted through the removed RoleBindings can last that long.

5. **Members of the `Auth` CR `adminGroups` or a DCH admin group manage connection types and connections** using the REST API with `X-Tenant-Id: global-assets`.

   > **Note:** See [Setting up a DCH admin group](concepts-terminologies.md#setting-up-a-dch-admin-group).

   ```python
   from data_connect_hub import CredentialsRef, DataConnectClient

   with DataConnectClient(endpoint="<gateway-host>", token="<token>", tenant_id="global-assets") as client:
       # Create a connection type in the global tenant; it is not visible from other tenants.
       client.create_connection_type(name="my-type", provider="postgres")
       # Create a shared connection; its Secret must exist in the global tenant.
       client.create_connection(name="shared-orders", connection_type_id="<type-id>", data_format="tabular", credentials_ref=CredentialsRef(secret="shared-orders"))
   ```

6. **User from `allowedGroups` ingests data.** Uses the Flight API with `X-Tenant-Id: global-assets` and `X-Data-Connection-Id: <shared-connection-id>`.

   ```python
   from data_connect_hub import DataConnectClient

   with DataConnectClient(endpoint="<gateway-host>", token="<token>", tenant_id="global-assets") as client:
       table = client.read("SELECT * FROM my_table", connection_id="<shared-connection-id>")
   ```

## What users see

Assume the DCH namespace is `opendatahub` and the global tenant is `global-assets`.

| Request | Header | Returns |
|---|---|---|
| `GET /api/v1alpha1/data/connection-types` | `X-Tenant-Id: <tenant-namespace>` | *Assume the user has `get` on `data-connection-types` in `<tenant-namespace>`.*<br>• Connection types in `opendatahub`, including the built-in ones (no permission in `opendatahub` required)<br>• Connection types in `<tenant-namespace>` |
| `GET /api/v1alpha1/data/connection-types` | `X-Tenant-Id: global-assets` | *Assume the user is a member of the `Auth` CR `allowedGroups` or `adminGroups`.*<br>• Connection types in `opendatahub`, including the built-in ones (no permission in `opendatahub` required)<br>• Connection types in `global-assets` |
| `GET /api/v1alpha1/data/connections` | `X-Tenant-Id: <tenant-namespace>` | *Assume the user has `get` on `data-connections` in `<tenant-namespace>`.*<br>• Connections in `<tenant-namespace>` |
| `GET /api/v1alpha1/data/connections` | `X-Tenant-Id: global-assets` | *Assume the user is a member of the `Auth` CR `allowedGroups` or `adminGroups`.*<br>• Connections in `global-assets` |
