# E2E Tests

End-to-end tests for Data Connect Hub, driven by the Python SDK against a live deployment.

## 1. Prerequisites

- A running DCH deployment (Kind, OpenShift, or any Kubernetes cluster)
- `kubectl` configured and pointing at the target cluster
- Python 3.11+

## 2. Setup & Run

```bash
cd /path/to/data-connect-hub

# 1. Find the gateway host. REST and Flight are served on the same
#    host:port, so one value covers both.
DCH_NS=dch   # namespace where DCH services run
oc get route -n openshift-ingress data-science-gateway -o jsonpath='{.spec.host}'   # RHOAI
# oc get route -n opendatahub odh-gateway -o jsonpath='{.spec.host}'                # ODH

kubectl port-forward -n $DCH_NS svc/dch-flight-service 19090:9090 &  # Flight metrics (optional)
kubectl port-forward -n $DCH_NS svc/dch-rest-service 19091:9090 &    # REST metrics (optional)

# 2. Copy the example config and fill in your values
cp e2e/env.example e2e/env.local
vi e2e/env.local

# 3. Run (installs deps, prepares K8s resources, runs pytest)
./e2e/run-e2e.sh e2e/env.local
```

To pass extra pytest arguments:

```bash
./e2e/run-e2e.sh e2e/env.local -k test_health
./e2e/run-e2e.sh e2e/env.local --tb=short -x
```

The runner creates a tenant-local `dch-rest-service-secret-access` RoleBinding to the predefined ClusterRole of the same name. This grants the REST ServiceAccount `get`, `create`, `patch`, and `delete` on secrets in `DCH_TENANT_ID` for inline credentials, failed-creation cleanup, readiness, and export. It also grants Flight read access to configured connector secrets. Service RBAC setup runs even when `DCH_AUTH_TOKEN` is supplied; the Kubernetes identity running the script must be allowed to create the Roles and RoleBindings and grant their referenced permissions.

For tenancy tests (`tests/test_tenancy.py`), the runner also creates the neighbor tenant namespace `DCH_NEIGHBOR_TENANT_ID` (default `dch-e2e-neighbor`) and grants the e2e user the same `dch-read-write` and `dch-secret-export` access there, plus the REST ServiceAccount secret access. Tests create resources in `DCH_TENANT_ID` and access them with `X-Tenant-Id` set to the neighbor tenant, so any leak comes from missing tenant scoping rather than missing RBAC. When `DCH_AUTH_TOKEN` is supplied, grant that identity the same roles in the neighbor tenant.

For permission tests (`tests/test_rbac.py`), the runner creates three more ServiceAccounts in `DCH_TENANT_ID`, each holding a fixed slice of the `dataconnecthub.opendatahub.io` resources defined in ODH-ADR-0001: `dch-e2e-reader` (bound to `dch-read`), `dch-e2e-metadata-only` (`get`/`list` on `data-connections` and `data-connection-types`, no `data-store`), and `dch-e2e-data-store-only` (`get` on `data-store` only). The tests check that each REST endpoint and Flight enforce the resource and verb the ADR assigns to them. Supply `DCH_READER_AUTH_TOKEN`, `DCH_METADATA_ONLY_AUTH_TOKEN`, or `DCH_DATA_STORE_ONLY_AUTH_TOKEN` to use existing identities with the same permissions instead.

The DCH controller must install the predefined ClusterRoles before running the suite. Tenant RoleBindings are created by the runner, not the default DCH deployment. Passing E2E tests does not establish secret access in other tenants. See [REST service secret permissions](../docs/user-guide/auth.md#23-grant-rest-service-permission-to-manage-connection-secrets) for tenant setup and [troubleshooting](../docs/user-guide/auth.md#4-troubleshooting) for permission failures.

## 3. Important Files and Directories

| Path | Purpose |
|------|---------|
| `e2e/env.example` | Template for required and optional E2E settings. |
| `e2e/run-e2e.sh` | Installs test dependencies, prepares the E2E environment, and runs the default suite. |
| `e2e/tests/` | Default E2E suite, run by `e2e/run-e2e.sh`. |
| `e2e/scenarios/` | Optional deployment-state or operational scenarios. Not run by `e2e/run-e2e.sh`; run separately after preparing prerequisites with `e2e/.venv/bin/pytest e2e/scenarios/ -v`. |
