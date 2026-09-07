#!/usr/bin/env bash
# Shared variables and helpers for CI scripts.
# Source this file; do not execute directly.

# Paths — computed from this file's location, not the caller's $0.
CI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$CI_DIR/../.." && pwd)"
TEMP_DIR="${TEMP_DIR:-/tmp/dch-ci}"
mkdir -p "$TEMP_DIR"

# Cluster
KIND_CLUSTER_NAME="${KIND_CLUSTER_NAME:-dch-e2e}"

# Namespaces
SVC_NAMESPACE="${SVC_NAMESPACE:-dch}"
CONTROLLER_NAMESPACE="${CONTROLLER_NAMESPACE:-dch}"
TENANT_NAMESPACE="${TENANT_NAMESPACE:-dch-tenant}"
NO_ACCESS_NAMESPACE="${NO_ACCESS_NAMESPACE:-no-access-ns}"

# Gateway
GATEWAY_NAME="${GATEWAY_NAME:-dch-gateway}"
GATEWAY_NAMESPACE="${GATEWAY_NAMESPACE:-dch}"
GATEWAY_LOCAL_PORT="${GATEWAY_LOCAL_PORT:-18443}"

# Service names
FLIGHT_SERVICE_NAME="${FLIGHT_SERVICE_NAME:-dch-flight-service}"
REST_SERVICE_NAME="${REST_SERVICE_NAME:-dch-rest-service}"
DCS_NAME="${DCS_NAME:-default-dcs}"

# Service accounts
FLIGHT_SA_NAME="${FLIGHT_SA_NAME:-dch-flight-service-sa}"
REST_SA_NAME="${REST_SA_NAME:-dch-rest-service-sa}"
SA_TOKEN_AUDIENCE="${SA_TOKEN_AUDIENCE:-https://kubernetes.default.svc}"

# Container images
FLIGHT_IMAGE="${FLIGHT_IMAGE:-dch-flight:e2e}"
REST_IMAGE="${REST_IMAGE:-dch-rest:e2e}"
CONTROLLER_IMAGE="${CONTROLLER_IMAGE:-dch-controller:e2e}"
KUBE_RBAC_PROXY_IMAGE="${KUBE_RBAC_PROXY_IMAGE:-quay.io/opendatahub/odh-kube-rbac-proxy:odh-stable}"
POSTGRES_IMAGE="${POSTGRES_IMAGE:-docker.io/library/postgres:16}"

# Metrics
FLIGHT_METRICS_PORT="${FLIGHT_METRICS_PORT:-19090}"

# NodePorts (mapped to localhost via kind extraPortMappings)
GATEWAY_NODE_PORT="${GATEWAY_NODE_PORT:-30443}"
METRICS_NODE_PORT="${METRICS_NODE_PORT:-30090}"

# Datasource defaults
POSTGRES_SSL_MODE="${POSTGRES_SSL_MODE:-disable}"
NEO4J_HELM_RELEASE="${NEO4J_HELM_RELEASE:-neo4j}"
NEO4J_ADMIN_PASSWORD="${NEO4J_ADMIN_PASSWORD:-testpassword}"
NEO4J_USERNAME="${NEO4J_USERNAME:-dch_reader}"
NEO4J_PASSWORD="${NEO4J_PASSWORD:-dch_readonly}"
ES_HELM_RELEASE="${ES_HELM_RELEASE:-elasticsearch}"
ES_PASSWORD="${ES_PASSWORD:-testpassword}"
MINIO_RELEASE="${MINIO_RELEASE:-minio}"
MINIO_IMAGE="${MINIO_IMAGE:-quay.io/minio/minio:latest}"
MINIO_MC_IMAGE="${MINIO_MC_IMAGE:-quay.io/minio/mc:latest}"
MINIO_ROOT_USER="${MINIO_ROOT_USER:-minioadmin}"
MINIO_ROOT_PASSWORD="${MINIO_ROOT_PASSWORD:-minioadmin}"
MINIO_BUCKET="${MINIO_BUCKET:-e2e-test}"

# CI workflow controls
E2E_DATASOURCES="${E2E_DATASOURCES:-postgres s3 neo4j elasticsearch milvus uri}"

# has_datasource <name> — true if <name> is in E2E_DATASOURCES.
has_datasource() { [[ " $E2E_DATASOURCES " == *" $1 "* ]]; }

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
