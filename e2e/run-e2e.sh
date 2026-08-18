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
ENV_FILE="$SCRIPT_DIR/.env"

# -------------------------------------------------------------------
# 1. Install dependencies
# -------------------------------------------------------------------

echo "=== E2E Setup ==="

VENV_DIR="$SCRIPT_DIR/.venv"
if [[ ! -d "$VENV_DIR" ]]; then
    echo "[0/7] Creating virtualenv ..."
    python3 -m venv "$VENV_DIR"
fi

VENV_PYTHON="$VENV_DIR/bin/python3"
VENV_PYTEST="$VENV_DIR/bin/pytest"

if [[ ! -x "$VENV_PYTEST" ]]; then
    echo "[0/7] Installing dependencies ..."
    "$VENV_PYTHON" -m pip install --quiet \
        -e "$REPO_ROOT/sdk/python[flight]" \
        -e "$SCRIPT_DIR"
fi

# -------------------------------------------------------------------
# 2. K8s setup (same as former setup.sh)
# -------------------------------------------------------------------

kubectl cluster-info --request-timeout=10s >/dev/null 2>&1 || {
    echo "ERROR: cannot reach Kubernetes cluster" >&2; exit 1
}
kubectl get svc -n "$DCH_SERVICE_NAMESPACE" -l app.kubernetes.io/name=flight-service -o name >/dev/null 2>&1 || {
    echo "ERROR: flight-service not found in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
}
kubectl get svc -n "$DCH_SERVICE_NAMESPACE" -l app.kubernetes.io/name=rest-service -o name >/dev/null 2>&1 || {
    echo "ERROR: rest-service not found in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
}

# ---- create tenant namespace for tests, and a no-access namespace for auth tests ----
kubectl create namespace "$DCH_TENANT_ID" 2>/dev/null || true
kubectl create namespace "$DCH_NO_ACCESS_NAMESPACE" 2>/dev/null || true
echo "[1/8] Namespaces ready: $DCH_TENANT_ID, $DCH_NO_ACCESS_NAMESPACE"

# ---- create SAs: allowed user for normal tests, denied user for auth tests ----
if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
    kubectl create sa "$E2E_SA_NAME" -n "$DCH_TENANT_ID" 2>/dev/null || true
    kubectl create sa "$E2E_DENIED_SA_NAME" -n "$DCH_TENANT_ID" 2>/dev/null || true
    echo "[2/8] Service accounts ready: $E2E_SA_NAME, $E2E_DENIED_SA_NAME"
else
    echo "[2/8] Skipped SA creation (DCH_AUTH_TOKEN already set)"
fi

# ---- grant allowed SA RBAC access to call REST and Flight services ----
if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
    kubectl delete rolebinding e2e-dch-access -n "$DCH_TENANT_ID" --ignore-not-found >/dev/null
    kubectl create rolebinding e2e-dch-access \
        -n "$DCH_TENANT_ID" \
        --clusterrole=dch-read-write \
        --serviceaccount="${DCH_TENANT_ID}:${E2E_SA_NAME}" >/dev/null
    echo "[3/8] SA RBAC ready (REST + Flight)"
else
    echo "[3/8] Skipped RBAC (DCH_AUTH_TOKEN already set)"
fi

# ---- read PG database URL from dch-database-config secret for creating test connections ----
PG_INTERNAL_URL=$(kubectl get secret dch-database-config -n "$DCH_SERVICE_NAMESPACE" \
    -o jsonpath='{.data.secret-config\.toml}' 2>/dev/null | base64 -d 2>/dev/null | \
    grep url | sed 's/.*= *"//' | sed 's/"//' || true)
if [[ -z "$PG_INTERNAL_URL" ]]; then
    echo "ERROR: failed to read database URL from secret dch-database-config in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
fi

echo "[4/8] PG database URL loaded"

# ---- create PG credential secret for Flight SQL connections ----
kubectl create secret generic "$PG_SECRET" \
    -n "$DCH_TENANT_ID" \
    --from-literal="url=${PG_INTERNAL_URL}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# ---- create S3 credential secret for Flight S3 connections (optional) ----
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

# ---- grant Flight SA read access to credential secrets for query execution ----
SECRET_NAMES=("--resource-name=$PG_SECRET")
if [[ "$E2E_S3_ENABLED" == "true" ]]; then
    SECRET_NAMES+=("--resource-name=$S3_SECRET")
fi

kubectl create role e2e-flight-secret-read \
    -n "$DCH_TENANT_ID" \
    --verb=get --resource=secrets \
    "${SECRET_NAMES[@]}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

kubectl create rolebinding e2e-flight-secret-read-rb \
    -n "$DCH_TENANT_ID" \
    --role=e2e-flight-secret-read \
    --serviceaccount="${DCH_SERVICE_NAMESPACE}:${DCH_FLIGHT_SA}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

if [[ "$E2E_S3_ENABLED" == "true" ]]; then
    echo "[5/8] PG + S3 secrets + Flight RBAC ready"
else
    echo "[5/8] PG secret + Flight RBAC ready (S3 skipped — no AWS credentials)"
fi

# ---- seed PG test data for query tests via temporary pod ----
E2E_SEED_POD="e2e-pg-seed"
kubectl delete pod "$E2E_SEED_POD" -n "$DCH_SERVICE_NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

kubectl run "$E2E_SEED_POD" -n "$DCH_SERVICE_NAMESPACE" \
    --image="$DCH_POSTGRES_IMAGE" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    --env="PGURI=${PG_INTERNAL_URL}" \
    --command -- sh -c '
psql "$PGURI" -v ON_ERROR_STOP=1 <<SQL
CREATE SCHEMA IF NOT EXISTS dch_e2e;
DROP TABLE IF EXISTS dch_e2e.cities;
CREATE TABLE dch_e2e.cities (
    id   SERIAL PRIMARY KEY,
    name TEXT    NOT NULL,
    country TEXT NOT NULL,
    population INTEGER NOT NULL
);
INSERT INTO dch_e2e.cities (name, country, population) VALUES
    ('"'"'Tokyo'"'"',  '"'"'Japan'"'"',          13960000),
    ('"'"'London'"'"', '"'"'United Kingdom'"'"',  8982000),
    ('"'"'Paris'"'"',  '"'"'France'"'"',          2161000);
SQL
'

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$E2E_SEED_POD" \
    -n "$DCH_SERVICE_NAMESPACE" --timeout=120s || {
    kubectl logs "$E2E_SEED_POD" -n "$DCH_SERVICE_NAMESPACE" --tail=20 || true
    echo "ERROR: failed to seed test data" >&2; exit 1
}
kubectl delete pod "$E2E_SEED_POD" -n "$DCH_SERVICE_NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
echo "[6/8] Test data seeded"

# ---- generate SA tokens for test authentication ----
TOKEN_ARGS=(--duration=4h)
if [[ -n "$DCH_TOKEN_AUDIENCE" ]]; then
    TOKEN_ARGS+=(--audience="$DCH_TOKEN_AUDIENCE")
fi

if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
    DCH_AUTH_TOKEN=$(kubectl create token "$E2E_SA_NAME" -n "$DCH_TENANT_ID" "${TOKEN_ARGS[@]}")
    echo "[7/8] Allowed SA token generated (4h TTL)"
else
    echo "[7/8] Using provided DCH_AUTH_TOKEN"
fi

if [[ -z "${DCH_DENIED_AUTH_TOKEN:-}" ]]; then
    DCH_DENIED_AUTH_TOKEN=$(kubectl create token "$E2E_DENIED_SA_NAME" -n "$DCH_TENANT_ID" "${TOKEN_ARGS[@]}")
    echo "[8/8] Denied SA token generated (4h TTL)"
else
    echo "[8/8] Using provided DCH_DENIED_AUTH_TOKEN"
fi

# ---- write .env for pytest ----
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

if [[ -n "${DCH_FLIGHT_METRICS_URL:-}" ]]; then
    echo "DCH_FLIGHT_METRICS_URL=${DCH_FLIGHT_METRICS_URL}" >> "$ENV_FILE"
fi

if [[ "$E2E_S3_ENABLED" == "true" ]]; then
    cat >> "$ENV_FILE" <<EOF
DCH_S3_SECRET=${S3_SECRET}
DCH_S3_CSV_QUERY=datasets/dch-test-prompts.csv
DCH_S3_PARQUET_QUERY=datasets/dch-test-prompts.parquet
DCH_S3_JSONL_QUERY=datasets/dch-test-prompts.jsonl
EOF
fi

echo ""
echo "=== E2E Setup Complete ==="

# -------------------------------------------------------------------
# 3. Run tests
# -------------------------------------------------------------------

echo ""
echo "=== Running E2E Tests ==="
cd "$SCRIPT_DIR"
exec "$VENV_PYTEST" tests/ -v "$@"
