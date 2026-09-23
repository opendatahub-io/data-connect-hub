#!/usr/bin/env bash
# Stage 2: Build images, load into kind, deploy DCH and tenant datasources.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

# ===================================================================
# Build images
# ===================================================================

echo "=== Building images ==="

echo "--- Building flight-service ---"
docker build -t "$CI_FLIGHT_IMAGE" -f "$CI_REPO_ROOT/services/flight/Containerfile.konflux" "$CI_REPO_ROOT"

echo "--- Building rest-service ---"
docker build -t "$CI_REST_IMAGE" -f "$CI_REPO_ROOT/services/rest/Containerfile.konflux" "$CI_REPO_ROOT"

echo "--- Building dc-controller ---"
sed -E -i.bak 's/^([[:space:]]*imagePullPolicy:).*/\1 IfNotPresent/' \
    "$CI_REPO_ROOT/config/base/flight-service/deployment.yaml"
rm -f "$CI_REPO_ROOT/config/base/flight-service/deployment.yaml.bak"
sed -E -i.bak 's/^([[:space:]]*imagePullPolicy:).*/\1 IfNotPresent/' \
    "$CI_REPO_ROOT/config/base/rest-service/deployment.yaml"
rm -f "$CI_REPO_ROOT/config/base/rest-service/deployment.yaml.bak"
docker build -t "$CI_CONTROLLER_IMAGE" -f "$CI_REPO_ROOT/dc-controller/Containerfile.konflux" "$CI_REPO_ROOT"

# ===================================================================
# Load images into kind
# ===================================================================

echo "=== Loading images into kind ==="
kind load docker-image "$CI_FLIGHT_IMAGE" --name "$CI_KIND_CLUSTER_NAME"
kind load docker-image "$CI_REST_IMAGE" --name "$CI_KIND_CLUSTER_NAME"
kind load docker-image "$CI_CONTROLLER_IMAGE" --name "$CI_KIND_CLUSTER_NAME"

assert_deployment_image() {
    local deployment="$1"
    local namespace="$2"
    local container="$3"
    local expected_image="$4"
    local actual_image

    if ! actual_image="$(kubectl get deployment "$deployment" -n "$namespace" \
        -o "jsonpath={.spec.template.spec.containers[?(@.name=='${container}')].image}" 2>/dev/null)"; then
        echo "ERROR: failed to inspect image for deployment/${deployment} in namespace ${namespace}" >&2
        kubectl get deployment "$deployment" -n "$namespace" -o yaml 2>/dev/null || true
        return 1
    fi

    if [[ "$actual_image" != "$expected_image" ]]; then
        echo "ERROR: deployment/${deployment} container ${container} uses '${actual_image}', expected '${expected_image}'" >&2
        kubectl get deployment "$deployment" -n "$namespace" \
            -o 'jsonpath={range .spec.template.spec.containers[*]}{.name}{"="}{.image}{"\n"}{end}' \
            2>/dev/null || true
        return 1
    fi

    echo "Verified deployment/${deployment} container ${container} uses ${expected_image}"
}

# ===================================================================
# System PostgreSQL
# ===================================================================

echo "=== Deploying system PostgreSQL ==="

sys_pg_url="postgresql://${CI_SYS_PG_USER}:${CI_SYS_PG_PASSWORD}@${CI_SYS_PG_HOST}:5432/${CI_SYS_PG_DATABASE}"

sys_pg_args=(-n "$CI_SVC_NAMESPACE" -r "$CI_SYS_PG_HOST" -u "$CI_SYS_PG_USER" -p "$CI_SYS_PG_PASSWORD" -d "$CI_SYS_PG_DATABASE" -t "180s")
if [[ "$CI_SSL_ENABLED" == "true" ]]; then
    sys_pg_args+=(--ssl)
    sys_pg_url="${sys_pg_url}?sslmode=verify-ca"
fi
bash "$CI_REPO_ROOT/hack/install-postgresql.sh" "${sys_pg_args[@]}"

# dch-database-config secret
sys_pg_secret_args=()
if [[ "$CI_SSL_ENABLED" == "true" ]]; then
    kubectl get secret "${CI_SYS_PG_HOST}-tls" -n "$CI_SVC_NAMESPACE" \
        -o jsonpath='{.data.ca\.crt}' | base64 -d > "${CI_TEMP_DIR}/postgresql-ca.crt"
    sys_pg_secret_args+=(--from-file=postgresql-ca.crt="${CI_TEMP_DIR}/postgresql-ca.crt")
fi

dch_secret_config_file="${CI_TEMP_DIR}/dch-secret-config.toml"
cat > "$dch_secret_config_file" <<EOF
[database]
url = "${sys_pg_url}"
EOF

kubectl create secret generic dch-database-config -n "$CI_SVC_NAMESPACE" \
    --from-file=secret-config.toml="$dch_secret_config_file" \
    "${sys_pg_secret_args[@]}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
rm -f "$dch_secret_config_file"

# ===================================================================
# Service TLS secrets
# ===================================================================

echo "=== Creating service TLS secrets ==="

for svc in "$CI_REST_SERVICE_NAME" "$CI_FLIGHT_SERVICE_NAME"; do
    openssl req -x509 -nodes -newkey rsa:2048 \
        -keyout "${CI_TEMP_DIR}/${svc}-tls.key" \
        -out "${CI_TEMP_DIR}/${svc}-tls.crt" \
        -subj "/CN=${svc}.${CI_SVC_NAMESPACE}.svc" \
        -addext "basicConstraints=critical,CA:FALSE" \
        -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
        -addext "extendedKeyUsage=serverAuth" \
        -addext "subjectAltName=DNS:${svc}.${CI_SVC_NAMESPACE}.svc,DNS:${svc}.${CI_SVC_NAMESPACE}.svc.cluster.local,DNS:${svc}" \
        -days 365 2>/dev/null

    tls_secret_name="${svc#dch-}-tls"
    kubectl create secret tls "$tls_secret_name" -n "$CI_SVC_NAMESPACE" \
        --cert="${CI_TEMP_DIR}/${svc}-tls.crt" \
        --key="${CI_TEMP_DIR}/${svc}-tls.key" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null
done

kubectl create configmap dch-rest-service-ca -n "$CI_CONTROLLER_NAMESPACE" \
    --from-file=service-ca.crt="${CI_TEMP_DIR}/${CI_REST_SERVICE_NAME}-tls.crt" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# Flight service CA configmap (rest-to-flight mTLS)
kubectl create configmap dch-flight-service-ca -n "$CI_SVC_NAMESPACE" \
    --from-file=service-ca.crt="${CI_TEMP_DIR}/${CI_FLIGHT_SERVICE_NAME}-tls.crt" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

rm -f "${CI_TEMP_DIR}"/*-tls.key "${CI_TEMP_DIR}"/*-tls.crt

# ===================================================================
# dc-controller (Helm)
# ===================================================================

echo "=== Installing dc-controller ==="

controller_repo="${CI_CONTROLLER_IMAGE%:*}"
controller_tag="${CI_CONTROLLER_IMAGE##*:}"

helm upgrade --install dc-controller "$CI_REPO_ROOT/dc-controller/charts" \
    --namespace "$CI_CONTROLLER_NAMESPACE" \
    --set operandNamespace="$CI_SVC_NAMESPACE" \
    --set dataConnectService.enabled=false \
    --set controllerManager.image.pullPolicy=IfNotPresent \
    --set "controllerManager.image.repository=${controller_repo}" \
    --set "controllerManager.image.tag=${controller_tag}" \
    --set "relatedImages.flightService=${CI_FLIGHT_IMAGE}" \
    --set "relatedImages.restService=${CI_REST_IMAGE}" \
    --set "relatedImages.kubeRbacProxy=${CI_KUBE_RBAC_PROXY_IMAGE}" \
    --set serviceCA.enabled=true \
    --set serviceCA.configMapName=dch-rest-service-ca

kubectl rollout status deployment/dc-controller-manager -n "$CI_CONTROLLER_NAMESPACE" --timeout=300s

# ===================================================================
# DataConnectService CR
# ===================================================================

echo "=== Creating DataConnectService CR ==="

if [[ -z "${CI_ENABLED_CONNECTORS//[[:space:]]/}" ]]; then
    echo "ERROR: CI_ENABLED_CONNECTORS must contain at least one connector" >&2
    exit 1
fi

kubectl apply -n "$CI_SVC_NAMESPACE" -f - <<EOF
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: ${CI_DCS_CR_NAME}
spec:
  gateway:
    name: ${CI_GATEWAY_NAME}
    namespace: ${CI_GATEWAY_NAMESPACE}
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
    "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" \
    -n "$CI_SVC_NAMESPACE" \
    --timeout=180s; then
    kubectl get dataconnectservices.dataconnecthub.opendatahub.io "$CI_DCS_CR_NAME" -n "$CI_SVC_NAMESPACE" -o yaml || true
    echo "ERROR: DataConnectService did not become Ready" >&2
    exit 1
fi

echo "=== DataConnectService CR ==="
kubectl get "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" -n "$CI_SVC_NAMESPACE" -o yaml

echo "=== Waiting for DCH rollout ==="
kubectl rollout status "deployment/${CI_FLIGHT_SERVICE_NAME}" -n "$CI_SVC_NAMESPACE" --timeout=180s
kubectl rollout status "deployment/${CI_REST_SERVICE_NAME}" -n "$CI_SVC_NAMESPACE" --timeout=180s
kubectl get po -n "$CI_SVC_NAMESPACE"

echo "=== Verifying DCH deployment images ==="
assert_deployment_image \
    "dc-controller-manager" \
    "$CI_CONTROLLER_NAMESPACE" \
    "manager" \
    "$CI_CONTROLLER_IMAGE"
assert_deployment_image \
    "$CI_FLIGHT_SERVICE_NAME" \
    "$CI_SVC_NAMESPACE" \
    "${CI_DCS_CR_NAME}-flight" \
    "$CI_FLIGHT_IMAGE"
assert_deployment_image \
    "$CI_REST_SERVICE_NAME" \
    "$CI_SVC_NAMESPACE" \
    "rest-service" \
    "$CI_REST_IMAGE"

# ===================================================================
# Flight metrics NodePort (mapped to localhost via kind extraPortMappings)
# ===================================================================

echo "=== Creating flight metrics NodePort service ==="
kubectl apply -n "$CI_SVC_NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Service
metadata:
  name: flight-metrics-nodeport
spec:
  type: NodePort
  selector:
    app.kubernetes.io/name: ${CI_DCS_CR_NAME}-flight
  ports:
  - port: 9090
    targetPort: 9090
    nodePort: ${CI_FLIGHT_METRICS_NODE_PORT}
EOF

# ===================================================================
# REST metrics NodePort (mapped to localhost via kind extraPortMappings)
# ===================================================================

echo "=== Creating REST metrics NodePort service ==="
kubectl apply -n "$CI_SVC_NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Service
metadata:
  name: rest-metrics-nodeport
spec:
  type: NodePort
  selector:
    app.kubernetes.io/name: rest-service
  ports:
  - port: 9090
    targetPort: 9090
    nodePort: ${CI_REST_METRICS_NODE_PORT}
EOF


# ===================================================================
# Tenant data sources for E2E connectors
# ===================================================================

if has_connector postgres; then
    echo "=== Deploying tenant PostgreSQL ==="
    tenant_pg_args=(-n "$CI_TENANT_NAMESPACE" -r "$CI_TENANT_PG_HOST" -u "$CI_TENANT_PG_USER" -p "$CI_TENANT_PG_PASSWORD" -d "$CI_TENANT_PG_DATABASE" -t "180s")
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        tenant_pg_args+=(--ssl)
    fi
    bash "$CI_REPO_ROOT/hack/install-postgresql.sh" "${tenant_pg_args[@]}"
fi

if has_connector neo4j; then
    echo "=== Deploying tenant Neo4j ==="
    neo4j_args=(-n "$CI_TENANT_NAMESPACE" -r "$CI_TENANT_NEO4J_HELM_RELEASE" -p "$CI_TENANT_NEO4J_ADMIN_PASSWORD")
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        neo4j_args+=(--ssl)
    fi
    bash "$CI_REPO_ROOT/hack/install-neo4j.sh" "${neo4j_args[@]}"
fi

if has_connector elasticsearch; then
    echo "=== Deploying tenant Elasticsearch ==="
    elasticsearch_args=(-n "$CI_TENANT_NAMESPACE" -r "$CI_TENANT_ES_HELM_RELEASE" -p "$CI_TENANT_ES_PASSWORD")
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        elasticsearch_args+=(--ssl)
    fi
    bash "$CI_REPO_ROOT/hack/install-elasticsearch.sh" "${elasticsearch_args[@]}"
fi

if has_connector milvus; then
    echo "=== Deploying tenant Milvus ==="
    milvus_args=(-n "$CI_TENANT_NAMESPACE")
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        milvus_args+=(--ssl)
    fi
    bash "$CI_REPO_ROOT/hack/install-milvus.sh" "${milvus_args[@]}"
fi

if has_connector s3; then
    echo "=== Deploying tenant SeaweedFS (S3) ==="
    s3_args=(
        -n "$CI_TENANT_NAMESPACE"
        -r "$CI_TENANT_S3_RELEASE"
        -u "$CI_TENANT_S3_ACCESS_KEY"
        -p "$CI_TENANT_S3_SECRET_KEY"
        -b "$CI_TENANT_S3_BUCKET"
        -i "$CI_TENANT_S3_IMAGE"
        -m "$CI_TENANT_S3_CLIENT_IMAGE"
    )
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        s3_args+=(--ssl)
    fi
    bash "$CI_REPO_ROOT/hack/install-seaweedfs.sh" "${s3_args[@]}"
fi

echo "=== Deployment complete ==="
