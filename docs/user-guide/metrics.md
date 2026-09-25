# Prometheus Metrics

Data Connect Hub exposes Prometheus metrics for its REST and Flight services. Each service serves an unauthenticated HTTP endpoint at `/metrics` on a dedicated port. Metrics are disabled unless enabled in the service configuration.

## Configuration

Configure metrics in each service's `config.toml`:

```toml
[metrics]
enabled = true
address = "0.0.0.0"
port = 9090
```

| Setting | Description | Default |
| --- | --- | --- |
| `enabled` | Starts the Prometheus metrics endpoint. | `false` |
| `address` | IP address on which the endpoint listens. | `0.0.0.0` |
| `port` | TCP port on which the endpoint listens. | `9090` |

The supplied Kubernetes manifests enable metrics for both services and expose their `metrics` Service ports on `9090`.

## Accessing Metrics

For a cluster deployment, use the service directly from within the cluster or port-forward it locally:

```console
kubectl port-forward -n <namespace> svc/dch-rest-service 19091:9090
kubectl port-forward -n <namespace> svc/dch-flight-service 19090:9090

curl http://127.0.0.1:19091/metrics
curl http://127.0.0.1:19090/metrics
```

The endpoints use plain HTTP and are not protected by the REST service's kube-rbac-proxy. Limit network access to the metrics ports according to the cluster's monitoring and network policy requirements.

## OpenShift Monitoring

When the OpenShift monitoring APIs are available, the controller deploys the `config/overlays/openshift` overlay. Its `ServiceMonitor` selects DCH-managed Services and scrapes their named `metrics` ports at `/metrics` every 30 seconds. No additional ServiceMonitor is required for the REST or Flight service.

For clusters that do not provide the Prometheus Operator APIs, create an equivalent scrape configuration or ServiceMonitor in the platform's monitoring stack. The target service must use the named `metrics` port and the `/metrics` path.

## REST Service Metrics

The REST service records metrics for every request, including `/health` and requests rejected before an API handler runs.

### `dch_rest_requests_total`

**Type:** Counter. **Labels:** `method`, `route`, `status`. **Description:** Completed REST requests.

### `dch_rest_request_duration_seconds`

**Type:** Histogram. **Labels:** `method`, `route`, `status`. **Description:** Request handler duration in seconds. Streaming response bodies are not included after their initial setup.

### `dch_rest_requests_active`

**Type:** Gauge. **Labels:** `method`, `route`. **Description:** Requests currently executing in a handler.

The `route` label is the matched Actix route pattern, such as `/api/v1alpha1/data/connections/{id}`, rather than the raw request path. This prevents connection IDs and other request values from creating unbounded label cardinality. Requests that do not match a route use `route="unmatched"`. Unknown HTTP methods are reported as `method="OTHER"`.

## Flight Service Metrics

The Flight service records metrics for supported Flight SQL request paths.

### `dch_flight_requests_total`

**Type:** Counter. **Labels:** `method`, `operation`, `status`. **Description:** Completed Flight requests.

### `dch_flight_request_duration_seconds`

**Type:** Histogram. **Labels:** `method`, `operation`, `status`. **Description:** Flight request setup duration in seconds. Streaming response bodies are not included after their initial setup.

### `dch_flight_requests_active`

**Type:** Gauge. **Labels:** `method`, `operation`. **Description:** Flight requests currently executing.

`method` is `arrow.flight.protocol.FlightService/GetFlightInfo` or `arrow.flight.protocol.FlightService/DoGet`. `operation` is one of `sql_info`, `tables`, `statement`, or `binary`. `status` is `OK` for successful requests or the corresponding gRPC status code.

## Example Queries

```promql
# REST request rate by route and status.
sum by (route, status) (rate(dch_rest_requests_total[5m]))

# REST 95th percentile handler latency by route.
histogram_quantile(
  0.95,
  sum by (le, route) (rate(dch_rest_request_duration_seconds_bucket[5m]))
)

# REST 5xx error rate by route.
sum by (route) (rate(dch_rest_requests_total{status=~"5.."}[5m]))

# Flight request rate by operation and status.
sum by (operation, status) (rate(dch_flight_requests_total[5m]))

# Flight 95th percentile request setup duration by operation.
histogram_quantile(
  0.95,
  sum by (le, operation) (rate(dch_flight_request_duration_seconds_bucket[5m]))
)
```
