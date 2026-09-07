#!/usr/bin/env bash
# Stage 2: Build images, load into kind, deploy DCH and tenant datasources.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

# ===================================================================
# Build images
# ===================================================================

echo "=== Building images ==="

echo "--- Building flight-service ---"
docker build -t "$FLIGHT_IMAGE" -f "$REPO_ROOT/services/flight/Containerfile" "$REPO_ROOT"

echo "--- Building rest-service ---"
docker build -t "$REST_IMAGE" -f "$REPO_ROOT/services/rest/Containerfile" "$REPO_ROOT"

echo "--- Patching operand manifests for local images ---"
sed -i.bak 's/imagePullPolicy: Always/imagePullPolicy: IfNotPresent/g' \
    "$REPO_ROOT/config/base/flight-service/deployment.yaml" \
    "$REPO_ROOT/config/base/rest-service/deployment.yaml"
rm -f "$REPO_ROOT/config/base/flight-service/deployment.yaml.bak" \
      "$REPO_ROOT/config/base/rest-service/deployment.yaml.bak"

echo "--- Building dc-controller ---"
docker build -t "$CONTROLLER_IMAGE" -f "$REPO_ROOT/dc-controller/Containerfile.konflux" "$REPO_ROOT"

# ===================================================================
# Load images into kind
# ===================================================================

echo "=== Loading images into kind ==="
kind load docker-image "$FLIGHT_IMAGE" --name "$KIND_CLUSTER_NAME"
kind load docker-image "$REST_IMAGE" --name "$KIND_CLUSTER_NAME"
kind load docker-image "$CONTROLLER_IMAGE" --name "$KIND_CLUSTER_NAME"

# ===================================================================
# System PostgreSQL
# ===================================================================

echo "=== Deploying system PostgreSQL ==="

SYS_PG_USER="dch_user"
SYS_PG_PASSWORD="dch_password"
SYS_PG_DATABASE="dch_db"
SYS_PG_HOST="dch-postgres"
DB_URL="postgresql://${SYS_PG_USER}:${SYS_PG_PASSWORD}@${SYS_PG_HOST}:5432/${SYS_PG_DATABASE}"

pg_args=(-n "$SVC_NAMESPACE" -r "$SYS_PG_HOST" -u "$SYS_PG_USER" -p "$SYS_PG_PASSWORD" -d "$SYS_PG_DATABASE" -t "180s")
if [[ "$POSTGRES_SSL_MODE" != "disable" ]]; then
    pg_args+=(--ssl)
    DB_URL="${DB_URL}?sslmode=${POSTGRES_SSL_MODE}"
fi
bash "$REPO_ROOT/hack/install-postgresql.sh" "${pg_args[@]}"

# dch-database-config secret
secret_args=()
if [[ "$POSTGRES_SSL_MODE" == "verify-ca" || "$POSTGRES_SSL_MODE" == "verify-full" ]]; then
    kubectl get secret "${SYS_PG_HOST}-tls" -n "$SVC_NAMESPACE" \
        -o jsonpath='{.data.ca\.crt}' | base64 -d > "${TEMP_DIR}/postgresql-ca.crt"
    secret_args+=(--from-file=postgresql-ca.crt="${TEMP_DIR}/postgresql-ca.crt")
fi

db_secret_file="${TEMP_DIR}/dch-secret-config.toml"
cat > "$db_secret_file" <<EOF
[database]
url = "${DB_URL}"
EOF

kubectl create secret generic dch-database-config -n "$SVC_NAMESPACE" \
    --from-file=secret-config.toml="$db_secret_file" \
    "${secret_args[@]}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
rm -f "$db_secret_file"

# ===================================================================
# Service TLS secrets
# ===================================================================

echo "=== Creating service TLS secrets ==="

for svc in "$REST_SERVICE_NAME" "$FLIGHT_SERVICE_NAME"; do
    openssl req -x509 -nodes -newkey rsa:2048 \
        -keyout "${TEMP_DIR}/${svc}-tls.key" \
        -out "${TEMP_DIR}/${svc}-tls.crt" \
        -subj "/CN=${svc}.${SVC_NAMESPACE}.svc" \
        -addext "basicConstraints=critical,CA:FALSE" \
        -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
        -addext "extendedKeyUsage=serverAuth" \
        -addext "subjectAltName=DNS:${svc}.${SVC_NAMESPACE}.svc,DNS:${svc}.${SVC_NAMESPACE}.svc.cluster.local,DNS:${svc}" \
        -days 365 2>/dev/null

    # Secret names match what the controller expects: rest-service-tls / flight-service-tls
    tls_secret_name="${svc#dch-}-tls"
    kubectl create secret tls "$tls_secret_name" -n "$SVC_NAMESPACE" \
        --cert="${TEMP_DIR}/${svc}-tls.crt" \
        --key="${TEMP_DIR}/${svc}-tls.key" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null
done

# Flight service CA configmap (rest-to-flight mTLS)
kubectl create configmap dch-flight-service-ca -n "$SVC_NAMESPACE" \
    --from-file=service-ca.crt="${TEMP_DIR}/${FLIGHT_SERVICE_NAME}-tls.crt" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

rm -f "${TEMP_DIR}"/*-tls.key "${TEMP_DIR}"/*-tls.crt

# ===================================================================
# dc-controller (Helm)
# ===================================================================

echo "=== Installing dc-controller ==="

CONTROLLER_REPO="${CONTROLLER_IMAGE%:*}"
CONTROLLER_TAG="${CONTROLLER_IMAGE##*:}"

helm upgrade --install dc-controller "$REPO_ROOT/dc-controller/charts" \
    --namespace "$CONTROLLER_NAMESPACE" \
    --set operandNamespace="$SVC_NAMESPACE" \
    --set dataConnectService.enabled=false \
    --set controllerManager.image.pullPolicy=IfNotPresent \
    --set "controllerManager.image.repository=${CONTROLLER_REPO}" \
    --set "controllerManager.image.tag=${CONTROLLER_TAG}" \
    --set "relatedImages.flightService=${FLIGHT_IMAGE}" \
    --set "relatedImages.restService=${REST_IMAGE}" \
    --set "relatedImages.kubeRbacProxy=${KUBE_RBAC_PROXY_IMAGE}"

kubectl rollout status deployment/dc-controller-manager -n "$CONTROLLER_NAMESPACE" --timeout=300s

# ===================================================================
# DataConnectService CR
# ===================================================================

echo "=== Creating DataConnectService CR ==="

kubectl apply -n "$SVC_NAMESPACE" -f - <<EOF
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: ${DCS_NAME}
spec:
  gateway:
    name: ${GATEWAY_NAME}
    namespace: ${GATEWAY_NAMESPACE}
  restService:
    env:
      - name: RUST_LOG
        value: info
  flightService:
    env:
      - name: RUST_LOG
        value: info
EOF

if ! kubectl wait \
    --for=jsonpath='{.status.phase}'=Ready \
    "dataconnectservices.dataconnecthub.opendatahub.io/${DCS_NAME}" \
    -n "$SVC_NAMESPACE" \
    --timeout=180s; then
    kubectl get dataconnectservices.dataconnecthub.opendatahub.io "$DCS_NAME" -n "$SVC_NAMESPACE" -o yaml || true
    echo "ERROR: DataConnectService did not become Ready" >&2
    exit 1
fi

echo "=== Waiting for DCH rollout ==="
kubectl rollout status "deployment/${FLIGHT_SERVICE_NAME}" -n "$SVC_NAMESPACE" --timeout=180s
kubectl rollout status "deployment/${REST_SERVICE_NAME}" -n "$SVC_NAMESPACE" --timeout=180s
kubectl get po -n "$SVC_NAMESPACE"

# ===================================================================
# Flight metrics NodePort (mapped to localhost via kind extraPortMappings)
# ===================================================================

echo "=== Creating flight metrics NodePort service ==="
kubectl apply -n "$SVC_NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Service
metadata:
  name: flight-metrics-nodeport
spec:
  type: NodePort
  selector:
    app.kubernetes.io/name: flight-service
  ports:
  - port: 9090
    targetPort: 9090
    nodePort: ${METRICS_NODE_PORT}
EOF

# ===================================================================
# Tenant datasources
# ===================================================================

echo "=== Deploying tenant datasources (${E2E_DATASOURCES}) ==="

if has_datasource postgres; then
    echo "--- Tenant PostgreSQL ---"
    TENANT_PG_HOST="dch-tenant-postgres"
    tenant_pg_args=(-n "$TENANT_NAMESPACE" -r "$TENANT_PG_HOST" -u dch_tenant_user -p dch_tenant_password -d dch_tenant_db -t "180s")
    if [[ "$POSTGRES_SSL_MODE" != "disable" ]]; then
        tenant_pg_args+=(--ssl)
    fi
    bash "$REPO_ROOT/hack/install-postgresql.sh" "${tenant_pg_args[@]}"
fi

if has_datasource neo4j; then
    echo "--- Neo4j ---"
    bash "$REPO_ROOT/hack/install-neo4j.sh" -n "$TENANT_NAMESPACE" -r "$NEO4J_HELM_RELEASE" -p "$NEO4J_ADMIN_PASSWORD"
fi

if has_datasource elasticsearch; then
    echo "--- Elasticsearch ---"
    bash "$REPO_ROOT/hack/install-elasticsearch.sh" -n "$TENANT_NAMESPACE" -r "$ES_HELM_RELEASE" -p "$ES_PASSWORD"
fi

if has_datasource milvus; then
    echo "--- Milvus ---"
    bash "$REPO_ROOT/hack/install-milvus.sh" -n "$TENANT_NAMESPACE"
fi

if has_datasource s3; then
    echo "--- MinIO (S3) ---"
    docker pull "$MINIO_IMAGE"
    docker pull "$MINIO_MC_IMAGE"
    kind load docker-image "$MINIO_IMAGE" --name "$KIND_CLUSTER_NAME"
    kind load docker-image "$MINIO_MC_IMAGE" --name "$KIND_CLUSTER_NAME"
    bash "$REPO_ROOT/hack/install-minio.sh" \
        -n "$TENANT_NAMESPACE" \
        -r "$MINIO_RELEASE" \
        -u "$MINIO_ROOT_USER" \
        -p "$MINIO_ROOT_PASSWORD" \
        -b "$MINIO_BUCKET" \
        -i "$MINIO_IMAGE" \
        -m "$MINIO_MC_IMAGE"
fi

echo "=== Deployment complete ==="
