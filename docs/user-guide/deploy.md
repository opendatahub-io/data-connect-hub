# Deploying Data Connect Hub

Data Connect Hub is deployed via the `dc-controller` operator using Helm.
The operator handles rest-service, flight-service, networking, and lifecycle
management automatically.

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
secret the services use to connect.

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

Wait for the operator to be ready:

```console
oc get csv -n openshift-operators | grep cloudnative-pg
# Wait until PHASE shows "Succeeded"
```

Set the controller and operand namespaces, then create them:

```console
export CONTROLLER_NS=dc-controller-system
export NS=dch-services            # where services and postgres run
oc create namespace $CONTROLLER_NS
oc create namespace $NS

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

Wait for the cluster to be healthy, then create the `dch-database-config`
secret from the auto-generated credentials:

```console
# Wait for cluster
oc get cluster dch-postgres -n $NS -w
# Once "Cluster in healthy state", create the secret
URI=$(oc get secret dch-postgres-app -n $NS -o jsonpath='{.data.uri}' | base64 -d)
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

## Install

Using the namespaces from the database setup above:

```console
cd dc-controller

helm install dc-controller chart/ \
  --namespace $CONTROLLER_NS \
  --set operandNamespace=$NS
```

## Override images

To use custom images (e.g. for dev/testing with a private registry):

```console
helm install dc-controller chart/ \
  --namespace $CONTROLLER_NS \
  --set operandNamespace=$NS \
  --set controllerManager.image.repository=quay.io/YOUR_ORG/dc-controller \
  --set controllerManager.image.tag=latest \
  --set relatedImages.restService=quay.io/YOUR_ORG/rest-service:latest \
  --set relatedImages.flightService=quay.io/YOUR_ORG/flight-service:latest
```


Image resolution priority (highest wins):
1. CR spec override (`spec.restService.image`)
2. Controller env var (`RELATED_IMAGE_ODH_DATA_CONNECT_HUB_REST_IMAGE`)
3. Default (`quay.io/opendatahub/odh-data-connect-hub-rest:odh-stable`)

## Verify

```console
oc get pods -n $NS
oc get dchs default-dataconnectservice -n $NS
```

The `Phase` column progresses through `Progressing` to `Ready`.

## Customising via values.yaml

All `DataConnectService` CR fields are configurable through Helm values.
The CR is created automatically with `dataConnectService.enabled: true`
(the default).

```yaml
dataConnectService:
  enabled: true        # set false to skip CR creation

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

## Monitor status

```console
oc get dchs default-dataconnectservice -n $NS -o yaml
```

Conditions: `Ready`, `ProvisioningSucceeded`, `Degraded`.

## What gets created

| Resource | Name | Notes |
|----------|------|-------|
| Deployment | `rest-service` | HTTP API on port 8080 |
| Deployment | `flight-service` | Arrow Flight gRPC on port 50051 |
| Service | `rest-service` | ClusterIP, port 8080 |
| Service | `flight-service` | ClusterIP, port 50051 |
| ServiceAccount | `data-connect-hub-sa` | For rest-service |
| ServiceAccount | `flight-service-sa` | For flight-service |
| ConfigMap | `rest-service-config` | Server config (config.toml) |
| ConfigMap | `flight-service-config` | Server config (config.toml) |
| NetworkPolicy | `rest-service` | Ingress/egress rules |
| NetworkPolicy | `flight-service` | Ingress/egress rules |
| HTTPRoute | `data-connect-hub` | Routes traffic via gateway |

All resources have owner references back to the CR. The CR carries a
finalizer, so deleting the CR cleans up everything.

## Uninstall

### 1. Delete the DataConnectService CR

The CR carries a finalizer, so deleting it cleans up all managed
resources (deployments, services, configmaps, networkpolicies, etc.):

```console
oc delete dchs default-dataconnectservice -n $NS
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
oc delete namespace $NS
oc delete namespace $CONTROLLER_NS
```

## Verify services

```console
# REST health check
oc exec deploy/rest-service -n $NS -- \
  curl -s http://localhost:8080/api/v1/data/health

# Flight gRPC health (pod readiness uses built-in gRPC probe)
oc get pod -l app.kubernetes.io/name=flight-service -n $NS
```

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
  gateway/             # HTTPRoute for external traffic
  overlays/
    dev/               # Dev overlay aggregating base + gateway
```

### Deploy

```console
# Ensure dch-database-config secret exists first
oc apply -k config/overlays/dev -n <your-namespace>
oc rollout status deployment/rest-service -n <your-namespace>
oc rollout status deployment/flight-service -n <your-namespace>
```

Or deploy components individually:

```console
oc apply -k config/base/rest-service -n <your-namespace>
oc apply -k config/base/flight-service -n <your-namespace>
```

### Updating images

`imagePullPolicy: Always` is set on both services. To pick up a new image
pushed to the configured tag:

```console
oc rollout restart deployment/rest-service -n <your-namespace>
oc rollout restart deployment/flight-service -n <your-namespace>
```

---

## Known gaps

- **NetworkPolicy** resources allow all ingress/egress -- real restriction
  is pending a defined gateway/client topology.
- **Operand namespace** is not auto-created by the controller -- create it
  before installing if using `operandNamespace`.
