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

## 3. Important Files and Directories

| Path | Purpose |
|------|---------|
| `e2e/env.example` | Template for required and optional E2E settings. |
| `e2e/run-e2e.sh` | Installs test dependencies, prepares the E2E environment, and runs the default suite. |
| `e2e/tests/` | Default E2E suite, run by `e2e/run-e2e.sh`. |
| `e2e/scenarios/` | Optional deployment-state or operational scenarios. Not run by `e2e/run-e2e.sh`; run separately after preparing prerequisites with `e2e/.venv/bin/pytest e2e/scenarios/ -v`. |
