# Data Connect Hub Controller

Kubernetes operator for deploying and managing Data Connect Hub services
(rest-service, flight-service) on OpenShift / RHOAI clusters.

## Deployment

See [`docs/user-guide/deploy.md`](../docs/user-guide/deploy.md) for full
deployment instructions (Helm and Kustomize).

Quick start:

```console
cd dc-controller

helm install dc-controller charts/ \
  --namespace dc-controller-system --create-namespace
```

## Custom Resource

The `DataConnectService` CR is namespace-scoped. After installing the
operator, create the CR in the namespace where you want the services deployed:

```yaml
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
  namespace: opendatahub
spec:
  restService: {}
  flightService: {}
```

The controller requires a `dch-database-config` Secret in the same
namespace as the CR. Provision PostgreSQL (e.g., via CloudNativePG on
OpenShift or an external service) and create the secret before creating
the CR. See [deploy.md](../docs/user-guide/deploy.md) for full instructions.

### Status

The CR follows the ODH PlatformObject contract:

```yaml
status:
  phase: Ready
  conditions:
    - type: Ready
      status: "True"
    - type: ProvisioningSucceeded
      status: "True"
    - type: Degraded
      status: "False"
```

### Gateway configuration

| Platform | Gateway name | Namespace |
|----------|-------------|-----------|
| ODH | `odh-gateway` | `opendatahub` |
| RHOAI | `data-science-gateway` | `openshift-ingress` |

### Platform integration

When running under the ODH operator, platform configuration is delivered
via the `opendatahub-dataconnecthub-config` ConfigMap. The controller
watches this ConfigMap and reconciles on changes.

## Development

```console
make build          # compile
make test           # unit + controller tests (envtest)
make lint           # golangci-lint
make generate       # regenerate deepcopy
make manifests      # regenerate CRD + RBAC
make test-e2e       # e2e tests (requires Kind)
```

### Running locally

```console
make install        # install CRD
make run            # run controller against your cluster
```

## License

Apache License 2.0
