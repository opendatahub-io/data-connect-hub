# Deploying Data Connect Hub

Data Connect Hub is deployed in two steps:

1. **Install the operator** via Helm — deploys the controller, CRDs, and RBAC.
2. **Create the DataConnectService CR** — the admin creates the CR in the
   namespace where services should run. The controller watches all namespaces
   and provisions the operand resources (rest-service, flight-service,
   networking) in the CR's namespace.

---

## Prerequisites

- OpenShift 4.20+ (Kubernetes 1.33+) -- required for the native `grpc:`
  readiness/liveness probe type used by `flight-service`.
- Logged in to the target cluster (`oc login` / `oc whoami` should work).

### Image pulls

- `rest-service` / `flight-service` images live at
  `quay.io/opendatahub/odh-data-connect-hub-{rest,flight}:odh-stable`,
  built by Konflux CI with `imagePullPolicy: Always`.

### Database

Data Connect Hub requires a PostgreSQL database. Provision an instance
using one of the methods below, then create the `dch-database-config`
secret in the namespace where you will create the DataConnectService CR.

#### Option A: CloudNativePG on OpenShift (recommended for dev/test)

Install the CloudNativePG operator from OperatorHub:

```console
oc apply -f - <<'EOF'
apiVersion: operators.coreos.com/v1alpha1
kind: Subscription
metadata:
  name: cloudnative-pg
  namespace: openshift-operators
spec:
  channel: stable-v1
  name: cloudnative-pg
  source: certified-operators
  sourceNamespace: openshift-marketplace
  installPlanApproval: Automatic
EOF
```

Expected output:

```
subscription.operators.coreos.com/cloudnative-pg created
```

Wait for the operator to be ready:

```console
oc get csv -n openshift-operators | grep cloudnative-pg
# Wait until PHASE shows "Succeeded"
```

Expected output:

```
cloudnative-pg.v1.30.0   CloudNativePG   1.30.0   Succeeded
```

If the CSV does not appear, check whether the install plan needs manual
approval (this can happen even with `installPlanApproval: Automatic`):

```console
oc get installplan -n openshift-operators | grep cloudnative-pg
# If APPROVED shows "false", approve it:
oc patch installplan $(oc get installplan -n openshift-operators --no-headers | grep cloudnative-pg | awk '{print $1}') \
  -n openshift-operators --type=merge -p '{"spec":{"approved":true}}'
```

Set the namespaces. Data Connect Hub installs alongside an existing
ODH or RHOAI deployment — the services run in the same namespace as
the platform:

| Platform | `$NS` |
|----------|-------|
| RHOAI (OpenShift AI) | `redhat-ods-applications` |
| ODH (Open Data Hub) | `opendatahub` |

These namespaces are created by the platform operator, so they will
already exist — the `AlreadyExists` error is safe to ignore:

```console
export CONTROLLER_NS=dc-controller-system
export NS=redhat-ods-applications   # use "opendatahub" for ODH
oc create namespace $CONTROLLER_NS
oc create namespace $NS             # safe to ignore "AlreadyExists"
oc project $NS
```

Expected output:

```
namespace/dc-controller-system created
Error from server (AlreadyExists): namespaces "redhat-ods-applications" already exists
Now using project "redhat-ods-applications" on server "https://api.<cluster>:443".
```

Create the CloudNativePG cluster:

```console
oc apply -n $NS -f - <<'EOF'
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: dch-postgres
spec:
  instances: 1
  storage:
    size: 5Gi
  bootstrap:
    initdb:
      database: dataconnecthub
      owner: dch
EOF
```

Expected output:

```
cluster.postgresql.cnpg.io/dch-postgres created
```

Wait for the cluster to be healthy, then create the `dch-database-config`
secret from the auto-generated credentials:

```console
# Wait for cluster
oc get cluster dch-postgres -n $NS -w
# The STATUS column may briefly show "Instance Status Extraction Error"
# — this is normal during initialization. The cluster is ready once
# the READY column shows the expected instance count (e.g. "1").
# Once ready, create the secret
URI=$(oc get secret dch-postgres-app -n $NS -o jsonpath='{.data.uri}' | base64 -d)
# Optional: verify the URI looks correct
# echo "$URI"
oc apply -n $NS -f - <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: dch-database-config
stringData:
  DATABASE_URL: "$URI"
  secret-config.toml: |
    [database]
    url = "$URI"
EOF
```

Expected output:

```
secret/dch-database-config created
```

Optional — verify the secret contents:

```console
oc extract secret/dch-database-config -n $NS --to=-
```

#### Option B: External PostgreSQL (production)

Provision your own PostgreSQL instance (e.g., AWS RDS, Azure Database
for PostgreSQL, or any PostgreSQL-compatible service) and create the
secret manually:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: dch-database-config
stringData:
  DATABASE_URL: "postgresql://user:password@hostname:5432/dbname"
  secret-config.toml: |
    [database]
    url = "postgresql://user:password@hostname:5432/dbname"
```

The controller will set a `Degraded` condition if this secret is missing
or malformed.

---

## Step 1: Install the operator

```console
cd dc-controller

helm install dc-controller charts/ \
  --namespace $CONTROLLER_NS
```

Expected output:

```
NAME: dc-controller
LAST DEPLOYED: Tue Aug 18 20:38:08 2026
NAMESPACE: dc-controller-system
STATUS: deployed
REVISION: 1
DESCRIPTION: Install complete
TEST SUITE: None
```

### Override controller images

To use custom images (e.g. for dev/testing with a private registry):

```console
helm install dc-controller charts/ \
  --namespace $CONTROLLER_NS \
  --set controllerManager.image.repository=quay.io/YOUR_ORG/dc-controller \
  --set controllerManager.image.tag=latest \
  --set relatedImages.restService=quay.io/YOUR_ORG/rest-service:latest \
  --set relatedImages.flightService=quay.io/YOUR_ORG/flight-service:latest
```

## Step 2: Create the DataConnectService CR

Create the CR in the namespace where you want the services deployed.
The `dch-database-config` secret must already exist in this namespace.
Set the `gateway` section to match your platform:

| Platform | Gateway name | Namespace |
|----------|-------------|-----------|
| RHOAI | `data-science-gateway` | `openshift-ingress` |
| ODH | `odh-gateway` | `opendatahub` |

```console
oc apply -f - <<EOF
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
  namespace: $NS
spec:
  gateway:
    name: data-science-gateway       # use "odh-gateway" for ODH
    namespace: openshift-ingress      # use "opendatahub" for ODH
EOF
```

Expected output:

```
dataconnectservice.dataconnecthub.opendatahub.io/default-dataconnectservice created
```

### Customising the CR

All service configuration is done directly on the CR spec:

```yaml
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
  namespace: opendatahub
spec:
  restService:
    image: "my-registry/rest-service:v1.2.3"
    replicas: 3
    resources:
      requests:
        cpu: "200m"
        memory: "512Mi"
      limits:
        cpu: "2"
        memory: "1Gi"
    env:
      - name: RUST_LOG
        value: debug
    imagePullSecrets:
      - name: my-pull-secret

  flightService:
    image: "my-registry/flight-service:v1.2.3"
    replicas: 2

  gateway:
    name: my-gateway
    namespace: my-gateway-ns
```

Available `ServiceOverrides` fields: `image`, `replicas`, `resources`, `env`,
`envFrom`, `volumes`, `volumeMounts`, `imagePullSecrets`.

Image resolution priority (highest wins):
1. CR spec override (`spec.restService.image`)
2. Controller env var (`RELATED_IMAGE_ODH_DATA_CONNECT_HUB_REST_IMAGE`)
3. Default (`quay.io/opendatahub/odh-data-connect-hub-rest:odh-stable`)

## Verify

```console
oc get pods -n $CONTROLLER_NS
oc get pods -n $NS | grep -E "dch-rest|dch-flight|dch-postgres"
oc get dchs default-dataconnectservice -n $NS
```

Expected output (once ready):

```
dc-controller-manager-765b9b57d8-xxxxx  1/1     Running   0          2m

dch-flight-service-59d944f8f9-xxxxx                1/1     Running   0          30s
dch-rest-service-555bf7fc78-xxxxx                  2/2     Running   0          30s
dch-postgres-1                                     1/1     Running   0          3m

NAMESPACE                 NAME                         PHASE
redhat-ods-applications   default-dataconnectservice   Ready
```

The `Phase` column progresses through `Progressing` to `Ready`.

Verify the HTTPRoute exists and is accepted by the gateway:

```console
oc get httproute dch-data-connect-hub -n $NS
```

Expected output:

```
NAME                   HOSTNAMES   AGE
dch-data-connect-hub               5m
```

Check the route is accepted:

```console
oc get httproute dch-data-connect-hub -n $NS \
  -o jsonpath='{range .status.parents[*].conditions[*]}{.type}: {.status}{"\n"}{end}'
```

Expected output:

```
Accepted: True
ResolvedRefs: True
```

If `Accepted` shows `False`, see the [Troubleshooting](#httproute-not-accepted-by-gateway)
section below.

## Verify services

```console
# REST health check (direct, bypasses auth)
oc exec deploy/dch-rest-service -c rest-service -n $NS -- \
  curl -s http://localhost:8080/api/v1/data/health
```

Expected output:

```
{"service":"Data Connect Hub"}
```

```console
# Flight gRPC health (pod readiness uses built-in gRPC probe)
oc get pod -l app.kubernetes.io/name=flight-service -n $NS
```

Expected output:

```
NAME                                  READY   STATUS    RESTARTS   AGE
dch-flight-service-59d944f8f9-xxxxx   1/1     Running   0          6m
```

### Test through the gateway

Get the gateway's external route and test the API with a bearer token.
All API calls (except `/health`) require the `X-Tenant-Id` header set
to the target namespace:

```console
TOKEN=$(oc whoami -t)

# RHOAI
GATEWAY_URL=$(oc get route -n openshift-ingress data-science-gateway -o jsonpath='{.spec.host}')

# ODH
# GATEWAY_URL=$(oc get route -n opendatahub odh-gateway -o jsonpath='{.spec.host}')

echo "Gateway: https://$GATEWAY_URL"
```

```console
# Health check through gateway
curl -sk -H "Authorization: Bearer $TOKEN" \
  "https://$GATEWAY_URL/api/v1/data/health"
```

Expected output:

```
{"service":"Data Connect Hub"}
```

```console
# List connection types (global types visible to all tenants)
curl -sk -H "Authorization: Bearer $TOKEN" -H "X-Tenant-Id: $NS" \
  "https://$GATEWAY_URL/api/v1/data/connection-types"
```

Expected output (5 default types from IDCT + 3 from ConfigMap migration):

```
{"total_count":8,"items":[...]}
```

```console
# List connections
curl -sk -H "Authorization: Bearer $TOKEN" -H "X-Tenant-Id: $NS" \
  "https://$GATEWAY_URL/api/v1/data/connections"
```

Expected output:

```
{"total_count":0,"items":[]}
```

## Monitor status

```console
oc get dchs default-dataconnectservice -n $NS -o yaml
```

Conditions: `Ready`, `ProvisioningSucceeded`, `Degraded`.

## What gets created

| Resource | Name | Notes |
|----------|------|-------|
| Deployment | `dch-rest-service` | HTTP API on port 8080 |
| Deployment | `dch-flight-service` | Arrow Flight gRPC on port 50051 |
| Service | `dch-rest-service` | ClusterIP, port 8443 |
| Service | `dch-flight-service` | ClusterIP, port 50051 |
| ServiceAccount | `dch-data-connect-hub-sa` | For rest-service |
| ServiceAccount | `dch-flight-service-sa` | For flight-service |
| ConfigMap | `dch-rest-service-config` | Server config (config.toml) |
| ConfigMap | `dch-flight-service-config` | Server config (config.toml) |
| NetworkPolicy | `dch-rest-service` | Ingress/egress rules |
| NetworkPolicy | `dch-flight-service` | Ingress/egress rules |
| HTTPRoute | `dch-data-connect-hub` | Routes traffic via gateway |

All resources have owner references back to the CR and carry the `dch-`
name prefix. Deleting the CR cleans up everything.

### Gateway configuration

| Platform | Gateway name | Namespace |
|----------|-------------|-----------|
| ODH | `odh-gateway` | `opendatahub` |
| RHOAI | `data-science-gateway` | `openshift-ingress` |

### Platform integration

When running under the ODH operator, platform configuration is delivered
via the `opendatahub-dataconnecthub-config` ConfigMap. The controller
watches this ConfigMap and reconciles on changes.

---

## Option 2: Kustomize deployment (lightweight)

For lightweight deployments without the operator, apply Kustomize
manifests directly.

### Prerequisites

A PostgreSQL database must be available before deploying services.
Create the `dch-database-config` secret in your target namespace with
the connection details (see the [Database](#database) section above for
the secret format).

### Layout

```text
config/
  base/
    rest-service/      # HTTP API (actix-web), port 8080
    flight-service/    # Arrow Flight gRPC service, port 50051
    gateway/           # HTTPRoute for external traffic
  overlays/
    dev/               # Dev overlay aggregating base (includes gateway)
```

### Deploy

```console
# Ensure dch-database-config secret exists first
oc apply -k config/overlays/dev -n <your-namespace>
oc rollout status deployment/dch-rest-service -n <your-namespace>
oc rollout status deployment/dch-flight-service -n <your-namespace>
```

Or deploy components individually:

```console
oc apply -k config/base -n <your-namespace>
```

### Updating images

`imagePullPolicy: Always` is set on both services. To pick up a new image
pushed to the configured tag:

```console
oc rollout restart deployment/dch-rest-service -n <your-namespace>
oc rollout restart deployment/dch-flight-service -n <your-namespace>
```

---

## Troubleshooting

### HTTPRoute not accepted by gateway

After creating the DataConnectService CR, verify the HTTPRoute status:

```console
oc get httproute dch-data-connect-hub -n $NS \
  -o jsonpath='{range .status.parents[*].conditions[*]}{.type}: {.status} - {.reason} - {.message}{"\n"}{end}'
```

If `Accepted` shows `False` with reason `NotAllowedByListeners`, the
gateway does not permit HTTPRoutes from your namespace. The RHOAI
`data-science-gateway` defaults to allowing only `openshift-ingress` and
`redhat-ods-applications`.

Check which namespaces are allowed:

```console
oc get gateway data-science-gateway -n openshift-ingress \
  -o jsonpath='{.spec.listeners[0].allowedRoutes.namespaces.selector.matchExpressions[0].values[*]}'
```

To fix, first prevent the ODH operator from reverting your change:

```console
oc annotate gateway data-science-gateway -n openshift-ingress \
  opendatahub.io/managed=false --overwrite
```

Then add your namespace to the allowed list:

```console
oc patch gateway data-science-gateway -n openshift-ingress --type=json \
  -p '[{"op":"replace",
        "path":"/spec/listeners/0/allowedRoutes/namespaces/selector/matchExpressions/0/values",
        "value":["openshift-ingress","redhat-ods-applications","'"$NS"'"]}]'
```

Re-check the HTTPRoute status — `Accepted` should now be `True`. Remove
the `opendatahub.io/managed` annotation when you no longer need the
override.

### API calls return 400 Bad Request

The kube-rbac-proxy sidecar on `dch-rest-service` requires an
`X-Tenant-Id` header on every request to resolve namespace-scoped
authorization. Requests without it return `400 Bad Request`.

```console
# Correct usage — include X-Tenant-Id set to the target namespace
curl -H "Authorization: Bearer $TOKEN" \
     -H "X-Tenant-Id: $NS" \
     https://<gateway>/api/v1/data/connection-types
```

The `/health` endpoint is not proxied and does not require this header.

### DataConnectService stuck in Error / DatabaseSecretMissing

The controller requires a `dch-database-config` secret in the same
namespace as the CR. Check the status for details:

```console
oc get dchs default-dataconnectservice -n $NS -o yaml
```

Verify the secret exists and has the required keys:

```console
oc get secret dch-database-config -n $NS -o jsonpath='{.data}' | jq -r 'keys[]'
# Expected: DATABASE_URL, secret-config.toml
```

See the [Database](#database) section for the secret format.

---

## Uninstall

### 1. Delete the DataConnectService CR

The CR carries a finalizer, so deleting it cleans up all namespace-scoped
managed resources (deployments, services, configmaps, networkpolicies).
Cluster-scoped resources (ClusterRoles, ClusterRoleBindings) are also
cleaned up via owner references:

```console
oc delete dchs default-dataconnectservice -n $NS
```

If cluster-scoped resources are not garbage-collected (e.g. after a
forced deletion), clean them up manually:

```console
oc delete clusterrole,clusterrolebinding -l dataconnecthub.opendatahub.io/managed-by=dataconnectservice
```

### 2. Remove the operator

```console
helm uninstall dc-controller -n $CONTROLLER_NS
```

Helm does **not** delete CRDs on uninstall (safety measure). To remove
them manually:

```console
oc delete crd dataconnectservices.dataconnecthub.opendatahub.io
oc delete crd initdataconnectiontypes.dataconnecthub.opendatahub.io
oc delete crd initdataconnections.dataconnecthub.opendatahub.io
```

### 3. Remove the database (if using CloudNativePG)

```console
oc delete cluster dch-postgres -n $NS
oc delete secret dch-database-config -n $NS
```

To also remove the CloudNativePG operator:

```console
oc delete subscription cloudnative-pg -n openshift-operators
oc delete csv -n openshift-operators -l operators.coreos.com/cloudnative-pg.openshift-operators=
oc delete crd -l cnpg.io/reload=
```

### 4. Delete the namespaces (optional)

```console
oc delete namespace $CONTROLLER_NS
```

> **Note:** Do not delete `$NS` if it is `redhat-ods-applications` or
> `opendatahub` — these are managed by the platform operator.

---

## Known gaps

- **NetworkPolicy** resources allow all ingress/egress -- real restriction
  is pending a defined gateway/client topology.
