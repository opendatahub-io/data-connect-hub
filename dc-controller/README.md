# Data Connect Hub Controller

Kubernetes operator for deploying and managing Data Connect Hub services
(rest-service, flight-service) on OpenShift / RHOAI clusters.

## Prerequisites

- Go 1.26+
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

Two deployment methods are available. Both install the same resources
(CRD, RBAC, controller Deployment, platform ConfigMap).

### Option A: Helm (recommended)

```console
cd dc-controller

helm install dc-controller chart/ \
  --namespace dc-controller-system --create-namespace
```

Then create the CR (see [Custom Resource](#custom-resource) below):

```console
oc apply -f config/samples/components.platform.opendatahub.io_v1alpha1_dataconnecthub.yaml
```

To override images (e.g. for testing with a custom registry):

```console
helm install dc-controller chart/ \
  --namespace dc-controller-system --create-namespace \
  --set controllerManager.image.repository=quay.io/YOUR_ORG/data-connect-hub-controller \
  --set controllerManager.image.tag=latest \
  --set relatedImages.restService=quay.io/YOUR_ORG/data-connect-hub-rest:latest \
  --set relatedImages.flightService=quay.io/YOUR_ORG/data-connect-hub-flight:latest
```

### Option B: Kustomize (make deploy)

```console
cd dc-controller

make deploy IMG=ghcr.io/opendatahub-io/data-connect-hub/dc-controller:latest
oc apply -f config/samples/components.platform.opendatahub.io_v1alpha1_dataconnecthub.yaml
```

### Verify

```console
# Wait ~60s, then check
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
| Service | `postgres` | ClusterIP, port 5432 (devMode only) |
| ServiceAccount | `data-connect-hub-sa` | For rest-service |
| ServiceAccount | `flight-service-sa` | For flight-service |
| ConfigMap | `rest-service-config` | Server config (config.toml) |
| ConfigMap | `flight-service-config` | Server config (config.toml) |
| Secret | `postgres-credentials` | Auto-generated DB credentials (devMode only) |
| PVC | `postgres-data` | 5Gi, ReadWriteOnce (devMode only) |
| NetworkPolicy | `rest-service` | Ingress/egress rules |
| NetworkPolicy | `flight-service` | Ingress/egress rules |
| NetworkPolicy | `postgres` | Ingress/egress rules (devMode only) |
| HTTPRoute | `data-connect-hub` | Routes traffic via gateway |

All resources have owner references back to the CR. The CR itself
carries a finalizer (`components.platform.opendatahub.io/finalizer`),
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

# Check flight-service pod is ready (uses built-in gRPC health probe)
oc get pod -l app.kubernetes.io/name=flight-service -n dc-controller-system
```

## Uninstall

```console
# Delete CR (cleans up all managed resources via finalizer)
oc delete dch default-dataconnecthub

# Remove the operator — choose the method you used to install:
helm uninstall dc-controller -n dc-controller-system   # Helm
make undeploy                                           # Kustomize
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
