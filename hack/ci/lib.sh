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
CI_SVC_NAMESPACE="${CI_SVC_NAMESPACE:-dch}"
CI_CONTROLLER_NAMESPACE="${CI_CONTROLLER_NAMESPACE:-dch}"
CI_TENANT_NAMESPACE="${CI_TENANT_NAMESPACE:-dch-tenant}"
CI_NO_ACCESS_NAMESPACE="${CI_NO_ACCESS_NAMESPACE:-no-access-ns}"

# Gateway
CI_GATEWAY_NAME="${CI_GATEWAY_NAME:-dch-gateway}"
CI_GATEWAY_NAMESPACE="${CI_GATEWAY_NAMESPACE:-dch}"
CI_GATEWAY_LOCAL_PORT="${CI_GATEWAY_LOCAL_PORT:-18443}"

# Service names
CI_FLIGHT_SERVICE_NAME="${CI_FLIGHT_SERVICE_NAME:-dch-flight-service}"
CI_REST_SERVICE_NAME="${CI_REST_SERVICE_NAME:-dch-rest-service}"
CI_DCS_CR_NAME="${CI_DCS_CR_NAME:-default-dcs}"

# Service accounts
CI_FLIGHT_SA_NAME="${CI_FLIGHT_SA_NAME:-dch-flight-service-sa}"
CI_REST_SA_NAME="${CI_REST_SA_NAME:-dch-rest-service-sa}"
CI_SA_TOKEN_AUDIENCE="${CI_SA_TOKEN_AUDIENCE:-https://kubernetes.default.svc}"

# Container images
CI_FLIGHT_IMAGE="${CI_FLIGHT_IMAGE:-dch-flight:e2e}"
CI_REST_IMAGE="${CI_REST_IMAGE:-dch-rest:e2e}"
CI_CONTROLLER_IMAGE="${CI_CONTROLLER_IMAGE:-dch-controller:e2e}"
CI_KUBE_RBAC_PROXY_IMAGE="${CI_KUBE_RBAC_PROXY_IMAGE:-quay.io/opendatahub/odh-kube-rbac-proxy:odh-stable}"

# Metrics
CI_FLIGHT_METRICS_LOCAL_PORT="${CI_FLIGHT_METRICS_LOCAL_PORT:-19090}"

# NodePorts (mapped to localhost via kind extraPortMappings)
CI_GATEWAY_NODE_PORT="${CI_GATEWAY_NODE_PORT:-30443}"
CI_FLIGHT_METRICS_NODE_PORT="${CI_FLIGHT_METRICS_NODE_PORT:-30090}"

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

# Tenant Datasource: MinIO
CI_TENANT_MINIO_RELEASE="${CI_TENANT_MINIO_RELEASE:-minio}"
CI_TENANT_MINIO_IMAGE="${CI_TENANT_MINIO_IMAGE:-quay.io/minio/minio:latest}"
CI_TENANT_MINIO_MC_IMAGE="${CI_TENANT_MINIO_MC_IMAGE:-quay.io/minio/mc:latest}"
CI_TENANT_MINIO_ROOT_USER="${CI_TENANT_MINIO_ROOT_USER:-minioadmin}"
CI_TENANT_MINIO_ROOT_PASSWORD="${CI_TENANT_MINIO_ROOT_PASSWORD:-minioadmin}"
CI_TENANT_MINIO_BUCKET="${CI_TENANT_MINIO_BUCKET:-e2e-test}"

# CI workflow controls
CI_ENABLED_CONNECTORS="${CI_ENABLED_CONNECTORS:-postgres sqlite s3 elasticsearch neo4j milvus uri}"
CI_DISABLED_CONNECTORS="${CI_DISABLED_CONNECTORS:-}"
CI_SSL_ENABLED="${CI_SSL_ENABLED:-true}"

# has_connector <name> — true if <name> is in CI_ENABLED_CONNECTORS.
has_connector() { [[ " $CI_ENABLED_CONNECTORS " == *" $1 "* ]]; }

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
