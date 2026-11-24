# Data Connect Hub Controller

Kubernetes operator for deploying and managing Data Connect Hub services
(rest-service, flight-service) on OpenShift / RHOAI clusters.

## Database prerequisite

An administrator must provision PostgreSQL and a `dch-database-config` Secret
in the `DataConnectService` namespace. The Secret must contain a
`secret-config.toml` key with the application's database configuration.
The controller does not provision PostgreSQL or generate database credentials.
This remains a prerequisite for the planned DSC integration as well.

If the Secret or required key is missing, the application reports
`Ready=False` and `ProvisioningSucceeded=False` with reason
`DatabaseSecretMissing`; it retries after the prerequisite is supplied.

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

| Platform | Gateway name | Namespace |
|----------|-------------|-----------|
| ODH | `odh-gateway` | `opendatahub` |
| RHOAI | `data-science-gateway` | `openshift-ingress` |

### Platform integration

The platform places DCH services in `odh-data-connect-hub-infra` (ODH) or
`rhoai-data-connect-hub-infra` (RHOAI), overridable through
`spec.components.data.dataConnectHub.namespace` on the DSC. The controller
remains in the platform application namespace. PostgreSQL and
`dch-database-config` must be provisioned in the DCH infrastructure namespace.

`spec.gateway.routeNamespace` places the HTTPRoute in the allowed application
namespace. The controller sets explicit backend namespaces, creates one named
Service ReferenceGrant per backend, and cleans up cross-namespace routes through
its finalizer. Omission retains same-namespace routing for standalone installs.
The platform ConfigMap key `gateway.routeNamespace` supplies the same default.

Legacy ODH connection Secrets stay in their tenant namespaces. Migration discovers
REST through the namespaced DCH CR and stores references to the original Secrets,
not credential copies in the infrastructure namespace.

The planned DSC integration uses the existing Data module
(`feast-module-operator`), not a separate DCH platform module. The platform
projects `spec.components.data.dataConnectHub` into the Data module CR;
the module installs `dc-controller` and creates its namespaced
`DataConnectService` application CR. The application reports its own status;
the Data module aggregates it for the platform. DCH does not write DSC status.

The application controller already supports runtime metadata via the
`opendatahub-dataconnecthub-config` ConfigMap. It watches this ConfigMap and
reconciles applications in the same namespace on changes. Standalone users
can continue to supply this ConfigMap themselves. Gateway and authentication
settings can be supplied directly in the application CR spec.

See the [onboarding plan](../docs/dev-guide/dsc-data-onboarding-plan.md) for
the proposed ownership contract, cross-repository work, and downstream image
onboarding. The shared-module integration is not yet implemented end to end.

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
