#!/usr/bin/env bash
# e2e/run-e2e.sh — One-stop E2E test runner for Data Connect Hub.
#
# Reads configuration from a file, prepares K8s resources, installs
# dependencies, and runs pytest.
#
# Usage:
#   ./e2e/run-e2e.sh e2e/env.local
#   make e2e-test ENV=e2e/env.local

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# -------------------------------------------------------------------
# Parse input
# -------------------------------------------------------------------

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <config-file> [pytest-args...]" >&2
    echo "" >&2
    echo "  Copy e2e/env.example to e2e/env.local, fill in your values," >&2
    echo "  then run:  $0 e2e/env.local" >&2
    exit 1
fi

CONFIG_FILE="$1"
shift

if [[ ! -f "$CONFIG_FILE" ]]; then
    echo "ERROR: config file not found: $CONFIG_FILE" >&2
    exit 1
fi

# Source the config file (only export known variables)
set -a
# shellcheck source=/dev/null
source "$CONFIG_FILE"
set +a

# -------------------------------------------------------------------
# Validate required variables
# -------------------------------------------------------------------

: "${DCH_SERVICE_NAMESPACE:?DCH_SERVICE_NAMESPACE is required (set it in $CONFIG_FILE)}"
: "${DCH_REST_URL:?DCH_REST_URL is required (set it in $CONFIG_FILE)}"
: "${DCH_FLIGHT_URL:?DCH_FLIGHT_URL is required (set it in $CONFIG_FILE)}"

: "${DCH_TENANT_ID:?DCH_TENANT_ID is required (set it in $CONFIG_FILE)}"
: "${DCH_NO_ACCESS_NAMESPACE:?DCH_NO_ACCESS_NAMESPACE is required (set it in $CONFIG_FILE)}"
: "${DCH_FLIGHT_SA:?DCH_FLIGHT_SA is required (set it in $CONFIG_FILE)}"
DCH_TOKEN_AUDIENCE="${DCH_TOKEN_AUDIENCE:-}"
: "${DCH_INSECURE:?DCH_INSECURE is required (set it in $CONFIG_FILE)}"
: "${DCH_POSTGRES_IMAGE:?DCH_POSTGRES_IMAGE is required (set it in $CONFIG_FILE)}"

E2E_SA_NAME="e2e-user"
E2E_DENIED_SA_NAME="e2e-denied-user"
PG_SECRET="e2e-pg-creds"
S3_SECRET="e2e-s3-creds"
MILVUS_SECRET="e2e-milvus-creds"
ENV_FILE="$SCRIPT_DIR/.env"

# -------------------------------------------------------------------
# Setup: namespaces and service accounts
# -------------------------------------------------------------------

setup_namespaces() {
    kubectl create namespace "$DCH_TENANT_ID" 2>/dev/null || true
    kubectl create namespace "$DCH_NO_ACCESS_NAMESPACE" 2>/dev/null || true
}

setup_service_accounts() {
    if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
        kubectl create sa "$E2E_SA_NAME" -n "$DCH_TENANT_ID" 2>/dev/null || true
        kubectl create sa "$E2E_DENIED_SA_NAME" -n "$DCH_TENANT_ID" 2>/dev/null || true
    fi
}

setup_sa_rbac() {
    if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
        kubectl delete rolebinding e2e-dch-access -n "$DCH_TENANT_ID" --ignore-not-found >/dev/null
        kubectl create rolebinding e2e-dch-access \
            -n "$DCH_TENANT_ID" \
            --clusterrole=dch-read-write \
            --serviceaccount="${DCH_TENANT_ID}:${E2E_SA_NAME}" >/dev/null
    fi
}

# -------------------------------------------------------------------
# Setup: credential secrets
# -------------------------------------------------------------------

setup_pg_secret() {
    PG_INTERNAL_URL=$(kubectl get secret dch-database-config -n "$DCH_SERVICE_NAMESPACE" \
        -o jsonpath='{.data.secret-config\.toml}' 2>/dev/null | base64 -d 2>/dev/null | \
        grep url | sed 's/.*= *"//' | sed 's/"//' || true)
    if [[ -z "$PG_INTERNAL_URL" ]]; then
        echo "ERROR: failed to read database URL from secret dch-database-config" >&2
        return 1
    fi
    kubectl create secret generic "$PG_SECRET" \
        -n "$DCH_TENANT_ID" \
        --from-literal="url=${PG_INTERNAL_URL}" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

setup_s3_secret() {
    E2E_S3_ENABLED="false"
    if [[ -n "${AWS_ACCESS_KEY_ID:-}" && -n "${AWS_SECRET_ACCESS_KEY:-}" ]]; then
        kubectl create secret generic "$S3_SECRET" \
            -n "$DCH_TENANT_ID" \
            --from-literal="AWS_S3_ENDPOINT=${AWS_S3_ENDPOINT}" \
            --from-literal="AWS_DEFAULT_REGION=${AWS_DEFAULT_REGION}" \
            --from-literal="AWS_S3_BUCKET=${AWS_S3_BUCKET}" \
            --from-literal="AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID}" \
            --from-literal="AWS_SECRET_ACCESS_KEY=${AWS_SECRET_ACCESS_KEY}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_S3_ENABLED="true"
    fi
}

setup_milvus_secret() {
    E2E_MILVUS_ENABLED="false"
    if [[ -n "${DCH_MILVUS_URI:-}" ]]; then
        local -a args=(--from-literal="uri=${DCH_MILVUS_URI}")
        [[ -n "${DCH_MILVUS_TOKEN:-}" ]] && args+=(--from-literal="token=${DCH_MILVUS_TOKEN}")
        kubectl create secret generic "$MILVUS_SECRET" \
            -n "$DCH_TENANT_ID" \
            "${args[@]}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_MILVUS_ENABLED="true"
    fi
}

setup_flight_secret_rbac() {
    local -a secret_names=("--resource-name=$PG_SECRET")
    [[ "$E2E_S3_ENABLED" == "true" ]] && secret_names+=("--resource-name=$S3_SECRET")
    [[ "$E2E_MILVUS_ENABLED" == "true" ]] && secret_names+=("--resource-name=$MILVUS_SECRET")

    kubectl create role e2e-flight-secret-read \
        -n "$DCH_TENANT_ID" \
        --verb=get --resource=secrets \
        "${secret_names[@]}" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null

    kubectl create rolebinding e2e-flight-secret-read-rb \
        -n "$DCH_TENANT_ID" \
        --role=e2e-flight-secret-read \
        --serviceaccount="${DCH_SERVICE_NAMESPACE}:${DCH_FLIGHT_SA}" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

# -------------------------------------------------------------------
# Setup: seed test data
# -------------------------------------------------------------------

seed_pg_data() {
    bash "$(dirname "$0")/scripts/seed-postgresql-data.sh" \
        -u "$PG_INTERNAL_URL" -n "$DCH_SERVICE_NAMESPACE" -i "$DCH_POSTGRES_IMAGE"
}

seed_milvus_data() {
    [[ "$E2E_MILVUS_ENABLED" == "true" ]] || return 0
    bash "$(dirname "$0")/scripts/seed-milvus-data.sh" \
        -e "$DCH_MILVUS_URI" -n "$DCH_SERVICE_NAMESPACE"
}

# -------------------------------------------------------------------
# Setup: generate auth tokens
# -------------------------------------------------------------------

generate_tokens() {
    local -a token_args=(--duration=4h)
    [[ -n "$DCH_TOKEN_AUDIENCE" ]] && token_args+=(--audience="$DCH_TOKEN_AUDIENCE")

    if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
        DCH_AUTH_TOKEN=$(kubectl create token "$E2E_SA_NAME" -n "$DCH_TENANT_ID" "${token_args[@]}")
    fi
    if [[ -z "${DCH_DENIED_AUTH_TOKEN:-}" ]]; then
        DCH_DENIED_AUTH_TOKEN=$(kubectl create token "$E2E_DENIED_SA_NAME" -n "$DCH_TENANT_ID" "${token_args[@]}")
    fi
}

# -------------------------------------------------------------------
# Setup: write .env for pytest
# -------------------------------------------------------------------

write_env_file() {
    cat > "$ENV_FILE" <<EOF
DCH_REST_URL=${DCH_REST_URL}
DCH_FLIGHT_URL=${DCH_FLIGHT_URL}
DCH_TENANT_ID=${DCH_TENANT_ID}
DCH_NO_ACCESS_NAMESPACE=${DCH_NO_ACCESS_NAMESPACE}
DCH_AUTH_TOKEN=${DCH_AUTH_TOKEN}
DCH_DENIED_AUTH_TOKEN=${DCH_DENIED_AUTH_TOKEN}
DCH_INSECURE=${DCH_INSECURE}
DCH_PG_SECRET=${PG_SECRET}
EOF

    [[ -n "${DCH_FLIGHT_METRICS_URL:-}" ]] && \
        echo "DCH_FLIGHT_METRICS_URL=${DCH_FLIGHT_METRICS_URL}" >> "$ENV_FILE"

    if [[ "$E2E_S3_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<EOF
DCH_S3_SECRET=${S3_SECRET}
DCH_S3_CSV_QUERY=datasets/dch-test-prompts.csv
DCH_S3_PARQUET_QUERY=datasets/dch-test-prompts.parquet
DCH_S3_JSONL_QUERY=datasets/dch-test-prompts.jsonl
EOF
    fi

    if [[ "$E2E_MILVUS_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<'MILVUS_EOF'
DCH_MILVUS_SECRET=e2e-milvus-creds
MILVUS_EOF
    fi
}

# ===================================================================
# Main
# ===================================================================

echo "=== E2E Setup ==="

# 1. Install dependencies
VENV_DIR="$SCRIPT_DIR/.venv"
if [[ ! -d "$VENV_DIR" ]]; then
    echo "[1/9] Creating virtualenv ..."
    python3 -m venv "$VENV_DIR"
fi
VENV_PYTHON="$VENV_DIR/bin/python3"
VENV_PYTEST="$VENV_DIR/bin/pytest"
if [[ ! -x "$VENV_PYTEST" ]]; then
    echo "[1/9] Installing dependencies ..."
    "$VENV_PYTHON" -m pip install --quiet \
        -e "$REPO_ROOT/sdk/python[flight]" \
        -e "$SCRIPT_DIR"
fi

# 2. Verify cluster
kubectl cluster-info --request-timeout=10s >/dev/null 2>&1 || {
    echo "ERROR: cannot reach Kubernetes cluster" >&2; exit 1
}
kubectl get svc -n "$DCH_SERVICE_NAMESPACE" -l app.kubernetes.io/name=flight-service -o name >/dev/null 2>&1 || {
    echo "ERROR: flight-service not found in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
}
kubectl get svc -n "$DCH_SERVICE_NAMESPACE" -l app.kubernetes.io/name=rest-service -o name >/dev/null 2>&1 || {
    echo "ERROR: rest-service not found in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
}

# 3. K8s setup
setup_namespaces
echo "[2/9] Namespaces ready"

setup_service_accounts
echo "[3/9] Service accounts ready"

setup_sa_rbac
echo "[4/9] SA RBAC ready"

# 4. Credential secrets
setup_pg_secret
setup_s3_secret
setup_milvus_secret
setup_flight_secret_rbac

SECRETS_MSG="PG"
[[ "$E2E_S3_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG} + S3"
[[ "$E2E_MILVUS_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG} + Milvus"
echo "[5/9] ${SECRETS_MSG} secrets + Flight RBAC ready"

# 5. Seed test data
seed_pg_data
echo "[6/9] PG test data seeded"

seed_milvus_data
if [[ "$E2E_MILVUS_ENABLED" == "true" ]]; then
    echo "[7/9] Milvus test data seeded"
else
    echo "[7/9] Milvus seed skipped (DCH_MILVUS_URI not set)"
fi

# 6. Auth tokens
generate_tokens
echo "[8/9] Auth tokens ready"

# 7. Write .env
write_env_file
echo "[9/9] .env written"

echo ""
echo "=== E2E Setup Complete ==="

# -------------------------------------------------------------------
# Run tests
# -------------------------------------------------------------------

echo ""
echo "=== Running E2E Tests ==="
cd "$SCRIPT_DIR"
exec "$VENV_PYTEST" tests/ -v "$@"
