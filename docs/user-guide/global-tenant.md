# Global Tenant

By default, DCH connection types and connections in a tenant namespace are accessible only to DCH users authorized in that namespace. In a global tenant, members of the `Auth` CR `allowedGroups` and `adminGroups` can read its connection types and connections and ingest data from them by selecting the global tenant with `X-Tenant-Id`.

Connection types in a global tenant stay scoped to it: they are not visible from other tenants, and connections in other tenants cannot use them. To share a connection type with every tenant, create it in the DCH namespace. This keeps every connection working when a platform admin changes which namespace is the global tenant: a connection only ever references a type in its own tenant or in the DCH namespace, which never changes.

See [Concepts and Terminologies](concepts-terminologies.md) for the terms used in this guide.

## Workflow

1. **Platform admin installs DCH.**
   The DCH installer installs the built-in connection types in the DCH namespace (`opendatahub` in this guide) and adds it to `[global-connection-types]`, so every DCH user can see them.

   To view the setting:
   ```bash
   oc get configmap dch-flight-service-config -n opendatahub -o jsonpath='{.data.config\.toml}'
   oc get configmap dch-rest-service-config -n opendatahub -o jsonpath='{.data.config\.toml}'
   ```
   ```toml
   [global-connection-types]
   tenant-ids = ["opendatahub"]
   ```
   dc-controller manages `tenant-ids`, so manual edits are overwritten on the next reconcile.

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
     | Members of the Auth CR `adminGroups` | [`dch-read-write`](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/base/user-roles/clusterroles.yaml) | • read/write to connection types and connections<br>• ingest data via a connection<br>• no access to Secrets |
     | Members of the Auth CR `allowedGroups` | [`dch-read`](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/base/user-roles/clusterroles.yaml) | • read-only to connection types and connections<br>• ingest data via a connection<br>• no access to Secrets |
     | REST ServiceAccount | [`dch-rest-service-secret-access`](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/base/rest-service/data-access-rbac.yaml) | • read/write to Secrets |
     | Flight ServiceAccount | [`dch-flight-service-secret-access`](https://github.com/opendatahub-io/data-connect-hub/blob/main/config/base/flight-service/secret-access-clusterrole.yaml) | • read-only to Secrets |

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

   Removing the label reverts both. RoleBindings that administrators create under other names are not touched. Neither step changes `[global-connection-types]`, so designating or removing a global tenant does not restart the REST or Flight service. The services cache authorization decisions for `[auth] cache_ttl_secs` (60 seconds by default), so access granted through the removed RoleBindings can last that long.

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
