# E2E Tests

End-to-end tests for Data Connect Hub, driven by the Python SDK against a live deployment.

## 1. Prerequisites

- A running DCH deployment (Kind, OpenShift, or any Kubernetes cluster)
- `kubectl` configured and pointing at the target cluster
- Python 3.11+

## 2. Setup & Run

```bash
cd /path/to/data-connect-hub

# 1. Start port-forwards
DCH_NS=dch   # namespace where DCH services run
kubectl port-forward -n $DCH_NS svc/dch-flight-service 50051:50051 &
kubectl port-forward -n $DCH_NS svc/dch-rest-service 18443:8443 &
kubectl port-forward -n $DCH_NS svc/dch-flight-service 19090:9090 &  # metrics (optional)

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

## 3. Configuration

See `env.example` for all available settings. Required fields:

| Variable | Description |
|----------|-------------|
| `DCH_SERVICE_NAMESPACE` | Namespace where DCH services run |
| `DCH_REST_URL` | REST service URL (e.g. `https://127.0.0.1:18443`) |
| `DCH_FLIGHT_URL` | Flight gRPC URL (e.g. `grpc+tls://127.0.0.1:50051`) |
