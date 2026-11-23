# Data Connect Hub Operator

Kubernetes operator for the Data Connect Hub platform module and its
DataConnectService application workloads (REST and Flight) on OpenShift / RHOAI.

## Platform module and application APIs

- `DataConnectHub` (`components.platform.opendatahub.io/v1alpha1`) is the
  cluster-scoped singleton module API named `default-dataconnecthub`.
- `DataConnectService` (`dataconnecthub.opendatahub.io/v1alpha1`) remains a
  namespaced application API. The module controller creates the owned
  `default-dataconnectservice` in the applications namespace.
- The module controller refuses to adopt an independently owned DCS with the
  reserved name. Namespace changes clean up the old owned DCS before creating
  the new one; namespace-local secrets are not copied.

## Custom Resource Status

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

Platform-managed installations use the Gateway discovered by the parent
operator; DSC `dataConnectHub.gateway` can override it. ODH and RHOAI default
to `data-science-gateway` in `openshift-ingress`; XKS defaults to `rh-ai-gateway`
in namespace `rh-ai-gateway`.

### Platform integration

The parent operator creates `odh-dataconnecthub-config`, including the platform
version handshake. The platform-managed DCS reads that configuration through
its DataConnectHub owner reference. DSC `dataConnectHub.tokenReviewAudiences`
overrides service-account issuer discovery.

Standalone DataConnectService installations can continue to use
`opendatahub-dataconnecthub-config`; the application controller uses it as a
fallback when the parent-owned module ConfigMap is not applicable.

## Development

```console
make build          # compile
make test           # unit + controller tests (envtest)
make lint           # golangci-lint
make generate       # regenerate deepcopy
make manifests      # regenerate CRD + RBAC
make test-e2e       # e2e tests (requires Kind)
```

### Helm CRD upgrades

Helm installs CRDs from `charts/crds/` on first install but does not upgrade
already-installed CRDs. Before upgrading the chart to a release that changes a
CRD schema, apply the generated CRDs first, then upgrade the chart:

```console
kubectl apply -f dc-controller/charts/crds/
helm upgrade --install dc-controller dc-controller/charts --namespace <operator-namespace>
```

### Running locally

```console
make install        # install CRD
make run            # run controller against your cluster
```

## License

Apache License 2.0
