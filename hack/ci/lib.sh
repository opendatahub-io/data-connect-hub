#!/usr/bin/env bash
# Shared variables and helpers for CI scripts.
# Source this file; do not execute directly.

# Paths — computed from this file's location, not the caller's $0.
CI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CI_REPO_ROOT="$(cd "$CI_DIR/../.." && pwd)"
CI_TEMP_DIR="${CI_TEMP_DIR:-/tmp/dch-ci}"
mkdir -p "$CI_TEMP_DIR"

# Cluster
CI_KIND_CLUSTER_NAME="${CI_KIND_CLUSTER_NAME:-dch-e2e}"

# Namespaces
CI_SVC_NAMESPACE="${CI_SVC_NAMESPACE:-dch-service}"
CI_CONTROLLER_NAMESPACE="${CI_CONTROLLER_NAMESPACE:-dch-controller}"
CI_TENANT_NAMESPACE="${CI_TENANT_NAMESPACE:-dch-tenant}"
CI_NO_ACCESS_NAMESPACE="${CI_NO_ACCESS_NAMESPACE:-no-access-ns}"
CI_NEIGHBOR_TENANT_NAMESPACE="${CI_NEIGHBOR_TENANT_NAMESPACE:-dch-neighbor-tenant}"

# Gateway
CI_GATEWAY_NAME="${CI_GATEWAY_NAME:-dch-gateway}"
CI_GATEWAY_NAMESPACE="${CI_GATEWAY_NAMESPACE:-dch-gateway}"
CI_GATEWAY_LOCAL_PORT="${CI_GATEWAY_LOCAL_PORT:-18443}"

# Istio — installed from the official OCI charts at
# ghcr.io/istio/release/charts (the old istio-release.storage.googleapis.com
# helm repo is being retired and returns 403 during migration).
CI_ISTIO_VERSION="${CI_ISTIO_VERSION:-1.31.1}"

# DataConnectService CR name (must be set before flight service names)
CI_DCS_CR_NAME="${CI_DCS_CR_NAME:-default-dcs}"

# Service names — Flight resource names are derived from the CR name by the
# controller's renderFlightService renaming (flight-service → {crName}-flight).
CI_FLIGHT_SERVICE_NAME="${CI_FLIGHT_SERVICE_NAME:-dch-${CI_DCS_CR_NAME}-flight}"
CI_REST_SERVICE_NAME="${CI_REST_SERVICE_NAME:-dch-rest-service}"

# Service accounts
CI_FLIGHT_SA_NAME="${CI_FLIGHT_SA_NAME:-dch-${CI_DCS_CR_NAME}-flight-sa}"
CI_REST_SA_NAME="${CI_REST_SA_NAME:-dch-rest-service-sa}"
CI_SA_TOKEN_AUDIENCE="${CI_SA_TOKEN_AUDIENCE:-https://kubernetes.default.svc}"

# Container images
CI_FLIGHT_IMAGE="${CI_FLIGHT_IMAGE:-dch-flight:e2e}"
CI_REST_IMAGE="${CI_REST_IMAGE:-dch-rest:e2e}"
CI_CONTROLLER_IMAGE="${CI_CONTROLLER_IMAGE:-dch-controller:e2e}"
CI_KUBE_RBAC_PROXY_IMAGE="${CI_KUBE_RBAC_PROXY_IMAGE:-quay.io/opendatahub/odh-kube-rbac-proxy:odh-stable}"

# Metrics
CI_FLIGHT_METRICS_LOCAL_PORT="${CI_FLIGHT_METRICS_LOCAL_PORT:-19090}"
CI_REST_METRICS_LOCAL_PORT="${CI_REST_METRICS_LOCAL_PORT:-19091}"

# Tracing (OTEL Collector + Jaeger)
CI_JAEGER_LOCAL_PORT="${CI_JAEGER_LOCAL_PORT:-16686}"
CI_OTEL_COLLECTOR_IMAGE="${CI_OTEL_COLLECTOR_IMAGE:-docker.io/otel/opentelemetry-collector:0.120.0}"
CI_JAEGER_IMAGE="${CI_JAEGER_IMAGE:-docker.io/jaegertracing/jaeger:2.6.0}"

# NodePorts (mapped to localhost via kind extraPortMappings)
CI_GATEWAY_NODE_PORT="${CI_GATEWAY_NODE_PORT:-30443}"
CI_FLIGHT_METRICS_NODE_PORT="${CI_FLIGHT_METRICS_NODE_PORT:-30090}"
CI_REST_METRICS_NODE_PORT="${CI_REST_METRICS_NODE_PORT:-30091}"
CI_JAEGER_NODE_PORT="${CI_JAEGER_NODE_PORT:-30686}"

# System PostgreSQL
CI_SYS_PG_HOST="${CI_SYS_PG_HOST:-dch-postgres}"
CI_SYS_PG_USER="${CI_SYS_PG_USER:-dch_user}"
CI_SYS_PG_PASSWORD="${CI_SYS_PG_PASSWORD:-dch_password}"
CI_SYS_PG_DATABASE="${CI_SYS_PG_DATABASE:-dch_db}"

# Tenant Datasource: PostgreSQL
CI_TENANT_PG_HOST="${CI_TENANT_PG_HOST:-dch-tenant-postgres}"
CI_TENANT_PG_USER="${CI_TENANT_PG_USER:-dch_tenant_user}"
CI_TENANT_PG_PASSWORD="${CI_TENANT_PG_PASSWORD:-dch_tenant_password}"
CI_TENANT_PG_DATABASE="${CI_TENANT_PG_DATABASE:-dch_tenant_db}"

# Tenant Datasource: Milvus

# Tenant Datasource: Neo4j
CI_TENANT_NEO4J_HELM_RELEASE="${CI_TENANT_NEO4J_HELM_RELEASE:-neo4j}"
CI_TENANT_NEO4J_ADMIN_PASSWORD="${CI_TENANT_NEO4J_ADMIN_PASSWORD:-testpassword}"
CI_TENANT_NEO4J_USERNAME="${CI_TENANT_NEO4J_USERNAME:-dch_reader}"
CI_TENANT_NEO4J_PASSWORD="${CI_TENANT_NEO4J_PASSWORD:-dch_readonly}"

# Tenant Datasource: Elasticsearch
CI_TENANT_ES_HELM_RELEASE="${CI_TENANT_ES_HELM_RELEASE:-elasticsearch}"
CI_TENANT_ES_PASSWORD="${CI_TENANT_ES_PASSWORD:-testpassword}"

# Tenant Datasource: SeaweedFS-backed S3
CI_TENANT_S3_RELEASE="${CI_TENANT_S3_RELEASE:-seaweedfs}"
CI_TENANT_S3_IMAGE="${CI_TENANT_S3_IMAGE:-docker.io/chrislusf/seaweedfs:3.99}"
CI_TENANT_S3_CLIENT_IMAGE="${CI_TENANT_S3_CLIENT_IMAGE:-docker.io/amazon/aws-cli:2.31.0}"
CI_TENANT_S3_ACCESS_KEY="${CI_TENANT_S3_ACCESS_KEY:-s3admin}"
CI_TENANT_S3_SECRET_KEY="${CI_TENANT_S3_SECRET_KEY:-s3password}"
CI_TENANT_S3_BUCKET="${CI_TENANT_S3_BUCKET:-e2e-test}"

# CI workflow controls
CI_ENABLED_CONNECTORS="${CI_ENABLED_CONNECTORS:-postgres sqlite s3 elasticsearch neo4j milvus uri}"
CI_DISABLED_CONNECTORS="${CI_DISABLED_CONNECTORS:-}"
CI_SSL_ENABLED="${CI_SSL_ENABLED:-true}"

# Upgrade test — the released baseline that hack/ci/upgrade-test.sh upgrades
# from, following docs/user-guide/upgrade.md.
CI_UPGRADE_BASE_REPO="${CI_UPGRADE_BASE_REPO:-https://github.com/opendatahub-io/data-connect-hub.git}"
CI_UPGRADE_BASE_REF="${CI_UPGRADE_BASE_REF:-odh-3.6}"
CI_UPGRADE_BASE_TAG="${CI_UPGRADE_BASE_TAG:-odh-3.6}"
CI_UPGRADE_BASE_CONTROLLER_IMAGE="quay.io/opendatahub/odh-data-connect-hub-controller:${CI_UPGRADE_BASE_TAG}"
CI_UPGRADE_BASE_REST_IMAGE="quay.io/opendatahub/odh-data-connect-hub-rest:${CI_UPGRADE_BASE_TAG}"
CI_UPGRADE_BASE_FLIGHT_IMAGE="quay.io/opendatahub/odh-data-connect-hub-flight:${CI_UPGRADE_BASE_TAG}"
CI_UPGRADE_BASE_KUBE_RBAC_PROXY_IMAGE="${CI_UPGRADE_BASE_KUBE_RBAC_PROXY_IMAGE:-quay.io/opendatahub/odh-kube-rbac-proxy:v3.6.0}"
CI_UPGRADE_STATE_FILE="${CI_UPGRADE_STATE_FILE:-${CI_TEMP_DIR}/upgrade-state.json}"

# has_connector <name> — true if <name> is in CI_ENABLED_CONNECTORS.
has_connector() { [[ " $CI_ENABLED_CONNECTORS " == *" $1 "* ]]; }

# ---------------------------------------------------------------------------
# assert_deployment_image <deployment> <namespace> <container> <expected_image>
# ---------------------------------------------------------------------------
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

# ---------------------------------------------------------------------------
# deploy_system_postgres — deploy the DCH metadata store PostgreSQL and the
# dch-database-config secret into $CI_SVC_NAMESPACE.
# ---------------------------------------------------------------------------
deploy_system_postgres() {
    local sys_pg_url="postgresql://${CI_SYS_PG_USER}:${CI_SYS_PG_PASSWORD}@${CI_SYS_PG_HOST}:5432/${CI_SYS_PG_DATABASE}"

    local -a sys_pg_args=(-n "$CI_SVC_NAMESPACE" -r "$CI_SYS_PG_HOST" -u "$CI_SYS_PG_USER" -p "$CI_SYS_PG_PASSWORD" -d "$CI_SYS_PG_DATABASE" -t "180s")
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        sys_pg_args+=(--ssl)
        sys_pg_url="${sys_pg_url}?sslmode=verify-ca"
    fi
    bash "$CI_REPO_ROOT/hack/install-postgresql.sh" "${sys_pg_args[@]}"

    local -a sys_pg_secret_args=()
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        kubectl get secret "${CI_SYS_PG_HOST}-tls" -n "$CI_SVC_NAMESPACE" \
            -o jsonpath='{.data.ca\.crt}' | base64 -d > "${CI_TEMP_DIR}/postgresql-ca.crt"
        sys_pg_secret_args+=(--from-file=postgresql-ca.crt="${CI_TEMP_DIR}/postgresql-ca.crt")
    fi

    local dch_secret_config_file="${CI_TEMP_DIR}/dch-secret-config.toml"
    cat > "$dch_secret_config_file" <<EOF
[database]
url = "${sys_pg_url}"
EOF

    kubectl create secret generic dch-database-config -n "$CI_SVC_NAMESPACE" \
        --from-file=secret-config.toml="$dch_secret_config_file" \
        "${sys_pg_secret_args[@]}" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null
    rm -f "$dch_secret_config_file"
}

# ---------------------------------------------------------------------------
# create_service_tls_secrets — create the REST and Flight TLS secrets and the
# CA configmaps consumed by the controller-manager and the REST service
# (rest-to-flight mTLS).
# ---------------------------------------------------------------------------
create_service_tls_secrets() {
    local svc tls_secret_name
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
}

# ---------------------------------------------------------------------------
# create_gateway_destination_rules — Istio originates TLS to the REST and
# Flight backends. The rules live in $CI_SVC_NAMESPACE and must be recreated
# whenever that namespace is recreated.
# ---------------------------------------------------------------------------
create_gateway_destination_rules() {
    kubectl apply -n "$CI_SVC_NAMESPACE" -f - <<EOF
apiVersion: networking.istio.io/v1
kind: DestinationRule
metadata:
  name: ${CI_REST_SERVICE_NAME}-tls
spec:
  host: ${CI_REST_SERVICE_NAME}.${CI_SVC_NAMESPACE}.svc.cluster.local
  trafficPolicy:
    portLevelSettings:
      - port:
          number: 8443
        tls:
          mode: SIMPLE
          insecureSkipVerify: true
---
apiVersion: networking.istio.io/v1
kind: DestinationRule
metadata:
  name: ${CI_FLIGHT_SERVICE_NAME}-tls
spec:
  host: ${CI_FLIGHT_SERVICE_NAME}.${CI_SVC_NAMESPACE}.svc.cluster.local
  trafficPolicy:
    portLevelSettings:
      - port:
          number: 8443
        tls:
          mode: SIMPLE
          insecureSkipVerify: true
EOF
}

# ---------------------------------------------------------------------------
# dump_cluster — print diagnostics for the given namespaces.
# Usage: dump_cluster ns1 ns2 ...
# ---------------------------------------------------------------------------
dump_cluster() {
    local namespaces=("$@")
    [[ ${#namespaces[@]} -eq 0 ]] && return

    for ns in "${namespaces[@]}"; do
        echo ""
        echo "######################################################################"
        echo "# Diagnostics for namespace: ${ns}"
        echo "######################################################################"

        echo ""
        echo "--- events (sorted by time) ---"
        kubectl get events -n "$ns" --sort-by='.lastTimestamp' 2>&1 || true

        echo ""
        echo "--- all workloads ---"
        kubectl get all -n "$ns" -o wide 2>&1 || true

        echo ""
        echo "--- pod details ---"
        kubectl get pods -n "$ns" -o yaml 2>&1 || true

        echo ""
        echo "--- pod logs ---"
        local pods
        pods=$(kubectl get pods -n "$ns" -o jsonpath='{.items[*].metadata.name}' 2>/dev/null) || true
        for pod in $pods; do
            local containers
            containers=$(kubectl get pod "$pod" -n "$ns" \
                -o jsonpath='{.spec.initContainers[*].name} {.spec.containers[*].name}' 2>/dev/null) || true
            for container in $containers; do
                echo ""
                echo "--- ${pod}/${container} logs ---"
                kubectl logs "$pod" -n "$ns" -c "$container" --tail=200 2>&1 || true
            done
        done
    done
}
