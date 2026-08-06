# Data Connect Hub Controller

Kubernetes operator for deploying and managing Data Connect Hub services
(rest-service, flight-service) on OpenShift / RHOAI clusters.

## Prerequisites

- Go 1.96+
- Access to an OpenShift 4.20+ cluster (Kubernetes 1.33+)
- `oc` CLI
- `podman` (logged into quay.io)
- Gateway API CRDs installed (present by default on RHOAI clusters)

## Quick Start — Local Development

```console
# Install the CRD
make install

# Run the controller locally against your cluster
make run

# In another terminal, apply a sample CR
oc apply -f config/samples/components.platform.opendatahub.io_v1alpha1_dataconnecthub.yaml
```

## Deploying to a Cluster

### Step 1: Build service images on-cluster

The Rust services need to be compiled for linux/amd64. Use OpenShift
BuildConfigs to build on-cluster (avoids QEMU issues on ARM Macs):

```console
cd data-connect-hub

# Create a namespace and push secret for quay.io
oc new-project dch-test 2>/dev/null || oc project dch-test
oc create secret docker-registry quay-push-secret \
  --from-file=.dockerconfigjson=$HOME/.config/containers/auth.json \
  -n dch-test

# Create BuildConfigs for both services
for svc in rest flight; do
cat <<EOF | oc apply -n dch-test -f -
apiVersion: build.openshift.io/v1
kind: BuildConfig
metadata:
  name: ${svc}-service-ubi9
spec:
  resources:
    limits: { cpu: "4", memory: 6Gi }
    requests: { cpu: "1", memory: 2Gi }
  source: { type: Binary }
  strategy:
    type: Docker
    dockerStrategy:
      dockerfilePath: services/${svc}/Containerfile
  output:
    to: { kind: DockerImage, name: "quay.io/<your-org>/data-connect-hub-${svc}:latest" }
    pushSecret: { name: quay-push-secret }
EOF
done

# Start builds (uploads repo as build context, ~6 min each)
oc start-build rest-service-ubi9 --from-dir=. -n dch-test --follow
oc start-build flight-service-ubi9 --from-dir=. -n dch-test --follow
```

### Step 2: Build and push the controller image

```console
cd dc-controller

# Cross-compile Go binary for amd64
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o manager-amd64 ./cmd/main.go

# Build container image (copies pre-compiled binary + manifests)
cd ..
podman build --platform linux/amd64 \
  -t quay.io/<your-org>/data-connect-hub-controller:latest \
  -f - . <<'DOCKERFILE'
FROM registry.access.redhat.com/ubi9/ubi-minimal:latest
WORKDIR /
COPY dc-controller/manager-amd64 /manager
COPY config/ /manifests/
USER 1001
ENTRYPOINT ["/manager"]
DOCKERFILE

podman push quay.io/<your-org>/data-connect-hub-controller:latest
```

### Step 3: Deploy the operator

```console
cd dc-controller

# Install CRD, RBAC, and controller deployment
make deploy IMG=quay.io/<your-org>/data-connect-hub-controller:latest

# Create a pull secret for quay.io images (named dch-pull-secret to match
# what the base manifests expect on the service accounts)
oc create secret docker-registry dch-pull-secret \
  --from-file=.dockerconfigjson=$HOME/.config/containers/auth.json \
  -n dc-controller-system

# Patch the controller to use your custom service images
oc patch deployment dc-controller-controller-manager \
  -n dc-controller-system --type=json -p='[
  {"op":"replace","path":"/spec/template/spec/containers/0/env/1/value",
   "value":"quay.io/<your-org>/data-connect-hub-rest:latest"},
  {"op":"replace","path":"/spec/template/spec/containers/0/env/2/value",
   "value":"quay.io/<your-org>/data-connect-hub-flight:latest"},
  {"op":"add","path":"/spec/template/spec/imagePullSecrets",
   "value":[{"name":"dch-pull-secret"}]}
]'

oc rollout status deployment/dc-controller-controller-manager \
  -n dc-controller-system --timeout=60s
```

### Step 4: Create a DataConnectHub CR

```console
oc apply -f - <<'EOF'
apiVersion: components.platform.opendatahub.io/v1alpha1
kind: DataConnectHub
metadata:
  name: default-dataconnecthub
spec:
  devMode: true
EOF

# Wait ~60s, then check status
oc get pods -n dc-controller-system
oc get dch default-dataconnecthub
```

## Custom Resource

The `DataConnectHub` CR is cluster-scoped and singleton (must be named
`default-dataconnecthub`). A minimal spec with defaults:

```yaml
apiVersion: components.platform.opendatahub.io/v1alpha1
kind: DataConnectHub
metadata:
  name: default-dataconnecthub
spec:
  devMode: true
```

This deploys rest-service, flight-service, and a dev Postgres instance
with default settings, plus an HTTPRoute targeting the default ODH gateway.

### Full spec reference

```yaml
apiVersion: components.platform.opendatahub.io/v1alpha1
kind: DataConnectHub
metadata:
  name: default-dataconnecthub
spec:
  devMode: true                        # default: true — deploys a single Postgres instance
  # database:
  #   externalSecret: my-db-secret     # required when devMode is false

  restService:
    image: quay.io/my-org/rest-service:v1.0
    replicas: 2
    resources:
      requests: { cpu: 100m, memory: 256Mi }
      limits:   { cpu: "1", memory: 512Mi }
    env:
      - name: RUST_LOG
        value: debug
    imagePullSecrets:
      - name: my-registry-secret

  flightService:
    image: quay.io/my-org/flight-service:v1.0
    replicas: 2
    imagePullSecrets:
      - name: my-registry-secret

  gateway:
    name: odh-gateway                  # default: odh-gateway
    namespace: opendatahub             # default: opendatahub
```

### Gateway configuration

The controller creates an HTTPRoute for external traffic routing.

| Platform | Gateway name | Namespace |
|----------|-------------|-----------|
| ODH | `odh-gateway` | `opendatahub` |
| RHOAI | `data-science-gateway` | `openshift-ingress` |

Gateway defaults can also be provided via the platform ConfigMap
(`opendatahub-dataconnecthub-config`) using keys `gateway.name` and
`gateway.namespace`. The CR spec takes precedence over ConfigMap values.

### What gets created

For each `DataConnectHub` CR, the controller creates:

| Resource | Name | Notes |
|----------|------|-------|
| Deployment | `rest-service` | HTTP API on port 8080 |
| Deployment | `flight-service` | Arrow Flight gRPC on port 50051 |
| Deployment | `postgres` | Only when `devMode: true` |
| Service | `rest-service` | ClusterIP, port 8080 |
| Service | `flight-service` | ClusterIP, port 50051 |
| Service | `postgres` | ClusterIP, port 5432 |
| ServiceAccount | `data-connect-hub-sa` | For rest-service |
| ServiceAccount | `flight-service-sa` | For flight-service |
| ConfigMap | `rest-service-config` | Server config (config.toml) |
| ConfigMap | `flight-service-config` | Server config (config.toml) |
| Secret | `postgres-credentials` | Auto-generated DB credentials |
| PVC | `postgres-data` | 5Gi, ReadWriteOnce |
| NetworkPolicy | `rest-service` | Ingress/egress rules |
| NetworkPolicy | `flight-service` | Ingress/egress rules |
| NetworkPolicy | `postgres` | Ingress/egress rules |
| HTTPRoute | `data-connect-hub` | Routes traffic via gateway |

All resources have owner references back to the CR and a finalizer,
so deleting the CR cleans up everything.

## Status

The CR follows the ODH PlatformObject contract:

```yaml
status:
  phase: Ready
  observedGeneration: 1
  distribution:
    name: Standalone        # or OpenDataHub, SelfManagedRHOAI
    version: 0.1.0
  releases:
    - name: rest-service
      repoUrl: https://github.com/opendatahub-io/data-connect-hub
      version: 0.1.0
    - name: flight-service
      repoUrl: https://github.com/opendatahub-io/data-connect-hub
      version: 0.1.0
    - name: platform          # only when managed by ODH operator
      version: 2.20.0
  conditions:
    - type: Ready
      status: "True"
    - type: ProvisioningSucceeded
      status: "True"
    - type: Degraded
      status: "False"
```

### Platform integration

When running under the ODH operator, platform configuration is delivered
via the `opendatahub-dataconnecthub-config` ConfigMap. The controller
watches this ConfigMap and reconciles on changes. Supported keys:

| Key | Description |
|-----|-------------|
| `distribution.name` | Platform name (OpenDataHub, SelfManagedRHOAI) |
| `distribution.version` | Platform version |
| `platformVersion` | Triggers the platform version handshake |
| `gateway.name` | Default gateway name |
| `gateway.namespace` | Default gateway namespace |

The platform version handshake: the controller reads `platformVersion`
from the ConfigMap and writes it to `status.releases[name=platform]`
only after all operands are Ready, signalling to the orchestrator that
the upgrade is complete.

## Verification

```console
# Check the CR status
oc get dch default-dataconnecthub -o yaml

# Check pods
oc get pods -n dc-controller-system

# Test rest-service health
oc exec deploy/rest-service -n dc-controller-system -- \
  curl -s http://localhost:8080/api/v1/data/health

# Test flight-service gRPC health
oc exec deploy/flight-service -n dc-controller-system -- \
  grpc_health_probe -addr=localhost:50051
```

## Uninstall

```console
# Delete CR (cleans up all managed resources via finalizer)
oc delete dch default-dataconnecthub

# Remove the controller, RBAC, and CRD
cd dc-controller && make undeploy
```

## Development

```console
make build          # compile
make test           # unit + controller tests (envtest)
make lint           # golangci-lint
make generate       # regenerate deepcopy
make manifests      # regenerate CRD + RBAC
make test-e2e       # e2e tests (requires Kind)
```

## License

Apache License 2.0
