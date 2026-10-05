
## Configuration Reload

Both flight-service and rest-service read configuration, TLS certificates, and database secrets once at startup. They do not watch for file changes at runtime. The dc-controller recomputes a content hash of all mounted ConfigMaps and Secrets on each reconcile cycle and triggers a rolling restart when the hash changes. Changes to externally-managed resources are detected on the next periodic reconcile (approximately 5 minutes).

### How it works

The dc-controller computes a SHA-256 content hash from ConfigMap and Secret data referenced by each Deployment's `configMap` and `secret` volumes, and stamps the hash as a `dataconnecthub/config-hash` annotation on the pod template. When the hash changes, the pod template updates and Kubernetes triggers a rolling update.

For controller-rendered ConfigMaps (e.g. `flight-service-config`, `rest-service-config`), the hash is computed from the desired state during reconciliation. For externally-managed resources (OpenShift serving-cert Secrets, CA-bundle ConfigMaps, database Secret), the controller fetches the live data from the cluster.

### Resources included in the content hash

| Resource | Type | Affected Deployment |
|---|---|---|
| `flight-service-config` | ConfigMap | flight-service |
| `flight-service-tls` | Secret (OpenShift serving-cert) | flight-service |
| `dch-database-config` | Secret | flight-service, rest-service |
| `rest-service-config` | ConfigMap | rest-service |
| `rest-service-tls` | Secret (OpenShift serving-cert) | rest-service |
| `flight-service-ca` | ConfigMap (OpenShift inject-cabundle) | rest-service |
| `rest-service-kube-rbac-proxy-config` | ConfigMap | rest-service |

### OpenShift certificate rotation

On OpenShift, TLS certificates are managed automatically:

- `flight-service-tls` and `rest-service-tls` are created and rotated by the service-ca-operator via the `service.beta.openshift.io/serving-cert-secret-name` annotation on each Service.
- `flight-service-ca` is populated by the service-ca-operator via the `service.beta.openshift.io/inject-cabundle` annotation on the ConfigMap.

When the service-ca-operator rotates a certificate, the controller detects the change on the next periodic reconcile, recomputes the content hash, and rolls out new pods.

### Limitations

- **Detection delay**: externally-managed resources (Secrets and CA-bundle ConfigMaps) are not actively watched. Changes are detected on the next periodic reconcile (approximately 5 minutes).
- **Custom ConfigMaps and Secrets** added via `ServiceOverrides` are included in the hash computation but follow the same periodic detection.
- **Projected volumes** are not included in the content hash. If custom `projected` volumes are added via `ServiceOverrides`, changes to their sources will not trigger a rolling restart.

### Graceful shutdown

Both deployments are configured with:

- `terminationGracePeriodSeconds: 30` — Kubernetes waits up to 30 seconds for the pod to shut down before sending SIGKILL.
- `preStop: exec: command: ["sleep", "3"]` — a 3-second delay before SIGTERM, allowing time for the pod to be removed from Service endpoints before the process begins draining.

Both Rust services handle SIGTERM gracefully: flight-service uses tonic's `serve_with_shutdown`, and rest-service uses actix-web's built-in signal handling (30-second worker shutdown timeout by default). The 30-second `terminationGracePeriodSeconds` includes the 3-second `preStop` hook, leaving 27 seconds for in-flight request draining before Kubernetes sends SIGKILL.
