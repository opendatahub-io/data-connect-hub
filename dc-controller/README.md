# Data Connect Hub Controller

Kubernetes operator for deploying and managing Data Connect Hub services
(rest-service, flight-service) on OpenShift / RHOAI clusters.

## Prerequisites

- Go 1.24+
- Access to an OpenShift 4.20+ cluster (Kubernetes 1.33+)
- `kubectl` or `oc` CLI
- Gateway API CRDs installed (present by default on RHOAI clusters)

## Quick Start — Local Development

```console
# Install the CRD
make install

# Run the controller locally against your cluster
make run

# In another terminal, apply a sample CR
kubectl apply -f config/samples/dataconnecthub_v1alpha1_dataconnectservice.yaml
```

## Deploying to a Cluster

Build, push, and deploy the controller as an in-cluster workload:

```console
make docker-build docker-push IMG=<your-registry>/dcc-controller:tag
make install
make deploy IMG=<your-registry>/dcc-controller:tag
```

Then create a `DataConnectService` CR:

```console
kubectl apply -f config/samples/dataconnecthub_v1alpha1_dataconnectservice.yaml
```

## Custom Resource

The `DataConnectService` CR controls what gets deployed. A minimal spec:

```yaml
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: my-dch
spec:
  description: "My Data Connect Hub instance"
```

This deploys rest-service, flight-service, and a dev Postgres instance with
default settings, plus an HTTPRoute targeting the default ODH gateway.

### Full spec reference

```yaml
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: my-dch
spec:
  description: "My Data Connect Hub instance"

  restService:
    image: ghcr.io/opendatahub-io/data-connect-hub/rest-service:latest
    replicas: 2
    resources:
      requests:
        cpu: 100m
        memory: 256Mi
      limits:
        cpu: "1"
        memory: 512Mi
    env:
      - name: RUST_LOG
        value: debug
    imagePullSecrets:
      - name: my-registry-secret

  flightService:
    image: ghcr.io/opendatahub-io/data-connect-hub/flight-service:latest
    replicas: 2
    resources:
      requests:
        cpu: 100m
        memory: 256Mi
    imagePullSecrets:
      - name: my-registry-secret

  database:
    devMode: true                    # default: true — deploys a single Postgres instance
    # externalSecret: my-db-secret   # when devMode: false, name of Secret with DB credentials

  gateway:
    name: data-science-gateway       # default: odh-gateway
    namespace: openshift-ingress     # default: opendatahub
```

### Gateway configuration

The controller creates an HTTPRoute for the rest-service, routing
`/v1/data/*` through the specified gateway.

| Platform | Gateway name | Namespace |
|----------|-------------|-----------|
| ODH | `odh-gateway` | `opendatahub` |
| RHOAI | `data-science-gateway` | `openshift-ingress` |

The HTTPRoute is only accepted if it's deployed in a namespace allowed by
the gateway's `allowedRoutes` selector. On RHOAI, this is typically
`redhat-ods-applications` and `openshift-ingress`.

### What gets created

For each `DataConnectService` CR, the controller creates:

| Resource | Name | Notes |
|----------|------|-------|
| Deployment | `rest-service` | HTTP API on port 8080 |
| Deployment | `flight-service` | Arrow Flight gRPC on port 50051 |
| Deployment | `postgres` | Only when `database.devMode: true` |
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
| HTTPRoute | `data-connect-hub` | Routes /v1/data to rest-service |

All resources have owner references back to the CR, so deleting the CR
cleans up everything.

## Status

The CR status reports the reconciliation state:

```yaml
status:
  phase: Ready
  httpRoute: data-connect-hub
  gateway:
    name: data-science-gateway
    namespace: openshift-ingress
  conditions:
    - type: Available
      status: "True"
    - type: Progressing
      status: "False"
    - type: Degraded
      status: "False"
```

## Verification

```console
# Check the CR status
kubectl get dataconnectservice my-dch -o yaml

# Check pods
kubectl get pods -l app.kubernetes.io/part-of=data-connect-hub

# Test rest-service health
kubectl exec deploy/rest-service -- curl -s http://localhost:8080/health

# Test via gateway (RHOAI — requires auth token)
TOKEN=$(oc whoami -t)
curl -sk -H "Authorization: Bearer $TOKEN" \
  https://<gateway-domain>/v1/data/connections
```

## Uninstall

```console
# Delete CR (cleans up all managed resources)
kubectl delete dataconnectservice my-dch

# Remove the controller
make undeploy

# Remove the CRD
make uninstall
```

## Development

```console
make build          # compile
make test           # unit + controller tests (envtest)
make lint           # clippy + fmt check
make generate       # regenerate deepcopy
make manifests      # regenerate CRD + RBAC
```

## License

Apache License 2.0
