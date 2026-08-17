# E2E Tests

End-to-end tests for Data Connect Hub, driven by the Python SDK against a live deployment.

## 1. Prerequisites

- A running DCH deployment (Kind, OpenShift, or any Kubernetes cluster)
- `kubectl` configured and pointing at the target cluster
- Python 3.11+

## 2. Setup

```bash
cd /path/to/data-connect-hub
export DCH_NS=dch   # namespace where DCH services run

# 1. Install test dependencies
make e2e-install

# 2. Start port-forwards
kubectl port-forward -n $DCH_NS svc/dch-flight-service 50051:50051 &
kubectl port-forward -n $DCH_NS svc/dch-rest-service 18443:8443 &

# 3. Prepare environment (once)
DCH_SERVICE_NAMESPACE=$DCH_NS \
DCH_REST_URL=https://127.0.0.1:18443 \
DCH_FLIGHT_URL=grpc+tls://127.0.0.1:50051 \
  bash e2e/setup.sh

# 4. Run tests (repeatable)
make e2e-test
```

## 3. Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DCH_SERVICE_NAMESPACE` | Yes | | Namespace where DCH services run |
| `DCH_REST_URL` | Yes | | REST service URL (e.g. `https://127.0.0.1:18443`) |
| `DCH_FLIGHT_URL` | Yes | | Flight gRPC URL (e.g. `grpc+tls://127.0.0.1:50051`) |
| `DCH_TENANT_ID` | No | `dch-e2e` | Tenant namespace name (created if missing) |
| `DCH_FLIGHT_SA` | No | `dch-flight-service-sa` | Flight service ServiceAccount name |
| `DCH_TOKEN_AUDIENCE` | No | `https://kubernetes.default.svc` | Audience for SA token |
| `DCH_INSECURE` | No | `true` | Skip TLS verification (for self-signed certs) |
| `DCH_AUTH_TOKEN` | No | *(generated)* | Skip SA/token creation if already set |
