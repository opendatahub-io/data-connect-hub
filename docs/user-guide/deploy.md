# Deploying Data Connect Hub

Data Connect Hub can be deployed in two ways:

1. **Operator (recommended)** -- install the `dc-controller` operator via Helm.
   The operator handles Postgres, rest-service, flight-service, networking, and
   lifecycle management automatically.
2. **Kustomize (manual)** -- apply the Kustomize manifests under `config/`
   directly. Useful when you want full control over each component or cannot
   run an operator.

---

## Prerequisites (both methods)

- OpenShift 4.20+ (Kubernetes 1.33+) -- required for the native `grpc:`
  readiness/liveness probe type used by `flight-service`.
- Logged in to the target cluster (`oc login` / `oc whoami` should work).

### Image pulls

- `rest-service` / `flight-service` images live at
  `ghcr.io/opendatahub-io/data-connect-hub/{rest-service,flight-service}:latest`,
  built by this repo's CI with `imagePullPolicy: Always`.
- `registry.redhat.io/rhel9/postgresql-16` requires registry auth, but on
  most OpenShift clusters the cluster-wide pull secret already covers
  `registry.redhat.io`.

---

## Option 1: Operator deployment (Helm)

A single `helm install` deploys the controller, CRD, RBAC, and creates
the `DataConnectHub` CR automatically.

### 1. Install

```console
cd dc-controller

helm install dc-controller chart/ \
  --namespace dc-controller-system --create-namespace
```

If you want operands (rest-service, flight-service, postgres) deployed to
a separate namespace, create it first and pass `operandNamespace`:

```console
oc create ns dch-services

helm install dc-controller chart/ \
  --namespace dc-controller-system --create-namespace \
  --set operandNamespace=dch-services
```

By default, operands are deployed into the controller's namespace.

### 2. Override images

To use custom images (e.g. for dev/testing with a private registry):

```console
helm install dc-controller chart/ \
  --namespace dc-controller-system --create-namespace \
  --set controllerManager.image.repository=quay.io/YOUR_ORG/dc-controller \
  --set controllerManager.image.tag=latest \
  --set relatedImages.restService=quay.io/YOUR_ORG/rest-service:latest \
  --set relatedImages.flightService=quay.io/YOUR_ORG/flight-service:latest
```

Image resolution priority (highest wins):
1. CR spec override (`spec.restService.image`)
2. Controller env var (`RELATED_IMAGE_ODH_DATA_CONNECT_HUB_REST_IMAGE`)
3. Default (`ghcr.io/opendatahub-io/data-connect-hub/rest-service:latest`)

### 3. Verify

```console
oc get pods -n dc-controller-system
oc get dch default-dataconnecthub
```

The `Phase` column progresses through `Progressing` to `Ready`.

### 4. Customising via values.yaml

All `DataConnectHub` CR fields are configurable through Helm values.
The CR is created automatically with `dataConnectHub.enabled: true`
(the default).

```yaml
dataConnectHub:
  enabled: true        # set false to skip CR creation
  # devMode: true      # omitted by default (CRD defaults to true)

  # External database (set devMode to false first)
  # database:
  #   externalSecret: my-database-secret

  restService:
    image: "my-registry/rest-service:v1.2.3"
    replicas: 3
    resources:
      requests:
        cpu: "200m"
        memory: "512Mi"
      limits:
        cpu: "2"
        memory: "1Gi"
    env:
      - name: RUST_LOG
        value: debug
    imagePullSecrets:
      - name: my-pull-secret

  flightService:
    image: "my-registry/flight-service:v1.2.3"
    replicas: 2

  gateway:
    name: my-gateway
    namespace: my-gateway-ns
```

Available `ServiceOverrides` fields: `image`, `replicas`, `resources`, `env`,
`envFrom`, `volumes`, `volumeMounts`, `imagePullSecrets`.

### 5. Monitor status

```console
oc get dch default-dataconnecthub -o yaml
```

Conditions: `Ready`, `ProvisioningSucceeded`, `Degraded`.

### 6. What gets created

| Resource | Name | Notes |
|----------|------|-------|
| Deployment | `rest-service` | HTTP API on port 8080 |
| Deployment | `flight-service` | Arrow Flight gRPC on port 50051 |
| Deployment | `postgres` | Only when `devMode: true` |
| Service | `rest-service` | ClusterIP, port 8080 |
| Service | `flight-service` | ClusterIP, port 50051 |
| Service | `postgres` | ClusterIP, port 5432 (devMode only) |
| ServiceAccount | `data-connect-hub-sa` | For rest-service |
| ServiceAccount | `flight-service-sa` | For flight-service |
| ConfigMap | `rest-service-config` | Server config (config.toml) |
| ConfigMap | `flight-service-config` | Server config (config.toml) |
| Secret | `postgres-credentials` | Auto-generated (devMode only) |
| PVC | `postgres-data` | 5Gi, ReadWriteOnce (devMode only) |
| NetworkPolicy | `rest-service` | Ingress/egress rules |
| NetworkPolicy | `flight-service` | Ingress/egress rules |
| NetworkPolicy | `postgres` | Ingress/egress rules (devMode only) |
| HTTPRoute | `data-connect-hub` | Routes traffic via gateway |

All resources have owner references back to the CR. The CR carries a
finalizer, so deleting the CR cleans up everything.

### 7. Uninstall

Delete the CR first so the finalizer cleans up all managed resources,
then remove the operator:

```console
oc delete dch default-dataconnecthub
helm uninstall dc-controller -n dc-controller-system
```

### 8. Verify

```console
# REST health check
oc exec deploy/rest-service -n <namespace> -- \
  curl -s http://localhost:8080/api/v1/data/health

# Flight gRPC health (pod readiness uses built-in gRPC probe)
oc get pod -l app.kubernetes.io/name=flight-service -n <namespace>
```

---

## Option 2: Kustomize deployment

### Layout

```text
config/
  base/
    rest-service/      # HTTP API (actix-web), port 8080
    flight-service/    # Arrow Flight gRPC service, port 50051
  db/
    postgres/          # Postgres instance backing both services
  overlays/
    dev/               # Dev overlay aggregating base + db + gateway
```

Each subdirectory under `base/` is a self-contained Kustomization. The
`overlays/dev` overlay aggregates all components for a single `oc apply`.

### 1. Generate Postgres credentials (once per environment)

Postgres credentials are **not** committed to the repo. Kustomize's
`secretGenerator` builds the `Secret` from local files generated by:

```console
./config/db/postgres/generate-secrets.sh
```

This creates (both `chmod 600`, git-ignored):
- `postgres-credentials.env` -- `POSTGRESQL_USER` / `POSTGRESQL_PASSWORD` /
  `POSTGRESQL_DATABASE`
- `secret-config.toml` -- `[database]` TOML with the full connection URL,
  mounted into rest-service/flight-service

The script refuses to overwrite existing files. Delete them first to rotate
credentials.

### 2a. Deploy everything at once (dev overlay)

```console
oc apply -k config/overlays/dev -n <your-namespace>
oc rollout status deployment/postgres -n <your-namespace>
oc rollout status deployment/rest-service -n <your-namespace>
oc rollout status deployment/flight-service -n <your-namespace>
```

### 2b. Deploy components individually

```console
# Postgres first
oc apply -k config/db/postgres -n <your-namespace>
oc rollout status deployment/postgres -n <your-namespace>

# Then services
oc apply -k config/base/rest-service -n <your-namespace>
oc apply -k config/base/flight-service -n <your-namespace>
oc rollout status deployment/rest-service -n <your-namespace>
oc rollout status deployment/flight-service -n <your-namespace>
```

### 3. Updating images

`imagePullPolicy: Always` is set on both services. To pick up a new image
pushed to `:latest`:

```console
oc rollout restart deployment/rest-service -n <your-namespace>
oc rollout restart deployment/flight-service -n <your-namespace>
```

---

## Known gaps

- **Postgres** is a single instance with a Kubernetes PVC -- fine for dev,
  not a substitute for real backups. For production, set `devMode: false`
  and provide `database.externalSecret` referencing a Secret with your
  database connection details.
- **NetworkPolicy** resources allow all ingress/egress -- real restriction
  is pending a defined gateway/client topology.
- **Operand namespace** is not auto-created by the controller -- create it
  before installing if using `operandNamespace`.
