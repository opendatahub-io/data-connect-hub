# Data Connect Hub Controller

Kubernetes operator for deploying and managing Data Connect Hub services
(rest-service, flight-service) on OpenShift / RHOAI clusters.

## Deployment

See [`docs/user-guide/deploy.md`](../docs/user-guide/deploy.md) for full
deployment instructions (Helm and Kustomize).

Quick start:

```console
cd dc-controller

helm install dc-controller chart/ \
  --namespace dc-controller-system --create-namespace
```

## Custom Resource

The `DataConnectHub` CR is cluster-scoped and singleton (must be named
`default-dataconnecthub`). Helm creates it automatically. A minimal spec:

```yaml
apiVersion: components.platform.opendatahub.io/v1alpha1
kind: DataConnectHub
metadata:
  name: default-dataconnecthub
spec: {}
```

`devMode` defaults to `true` (deploys a built-in Postgres). Set
`devMode: false` and provide `database.externalSecret` for an external
database.

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
