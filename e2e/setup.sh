#!/usr/bin/env bash
# e2e/setup.sh — prepare K8s environment for e2e tests.
#
# Run once before `make e2e-test`. Idempotent — safe to re-run.
# Exports env vars to a file that pytest reads automatically.
#
# Required env vars:
#   DCH_SERVICE_NAMESPACE   namespace where DCH services run
#   DCH_REST_URL            REST service URL (e.g. https://localhost:38443)
#   DCH_FLIGHT_URL          Flight gRPC URL (e.g. grpc+tls://localhost:35051)
#
# Optional:
#   DCH_TENANT_ID           tenant namespace name       (default: dch-e2e)
#   DCH_FLIGHT_SA           Flight service SA name      (default: dch-flight-service-sa)
#   DCH_TOKEN_AUDIENCE      SA token audience           (default: https://kubernetes.default.svc)
#   DCH_INSECURE            skip TLS verify for pytest  (default: true)
#   DCH_AUTH_TOKEN           skip SA/token creation if already set

set -euo pipefail

: "${DCH_SERVICE_NAMESPACE:?DCH_SERVICE_NAMESPACE is required}"
: "${DCH_REST_URL:?DCH_REST_URL is required}"
: "${DCH_FLIGHT_URL:?DCH_FLIGHT_URL is required}"

DCH_TENANT_ID="${DCH_TENANT_ID:-dch-e2e}"
DCH_FLIGHT_SA="${DCH_FLIGHT_SA:-dch-flight-service-sa}"
DCH_TOKEN_AUDIENCE="${DCH_TOKEN_AUDIENCE:-https://kubernetes.default.svc}"
DCH_INSECURE="${DCH_INSECURE:-true}"

E2E_SA_NAME="e2e-user"
E2E_PG_SECRET="e2e-pg-creds"
ENV_FILE="$(cd "$(dirname "$0")" && pwd)/.env"

echo "=== E2E Setup ==="

# ---- verify K8s connectivity and DCH services ----
kubectl cluster-info --request-timeout=10s >/dev/null 2>&1 || {
    echo "ERROR: cannot reach Kubernetes cluster" >&2; exit 1
}
kubectl get svc -n "$DCH_SERVICE_NAMESPACE" -l app.kubernetes.io/name=flight-service -o name >/dev/null 2>&1 || {
    echo "ERROR: flight-service not found in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
}
kubectl get svc -n "$DCH_SERVICE_NAMESPACE" -l app.kubernetes.io/name=rest-service -o name >/dev/null 2>&1 || {
    echo "ERROR: rest-service not found in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
}

# ---- 1. create tenant namespace ----
kubectl create namespace "$DCH_TENANT_ID" 2>/dev/null || true
echo "[1/7] Tenant namespace ready: $DCH_TENANT_ID"

# ---- 2. create SA ----
if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
    kubectl create sa "$E2E_SA_NAME" -n "$DCH_TENANT_ID" 2>/dev/null || true
    echo "[2/7] Service account ready: $E2E_SA_NAME"
else
    echo "[2/7] Skipped SA creation (DCH_AUTH_TOKEN already set)"
fi

# ---- 3. grant SA access to REST and Flight services ----
if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
    kubectl delete rolebinding e2e-dch-access -n "$DCH_TENANT_ID" --ignore-not-found >/dev/null
    kubectl create rolebinding e2e-dch-access \
        -n "$DCH_TENANT_ID" \
        --clusterrole=dch-read-write \
        --serviceaccount="${DCH_TENANT_ID}:${E2E_SA_NAME}" >/dev/null
    echo "[3/7] SA RBAC ready (REST + Flight)"
else
    echo "[3/7] Skipped RBAC (DCH_AUTH_TOKEN already set)"
fi

# ---- 4. read database URL from dch-database-config ----
PG_INTERNAL_URL=$(kubectl get secret dch-database-config -n "$DCH_SERVICE_NAMESPACE" \
    -o jsonpath='{.data.secret-config\.toml}' 2>/dev/null | base64 -d 2>/dev/null | \
    grep url | sed 's/.*= *"//' | sed 's/"//' || true)
if [[ -z "$PG_INTERNAL_URL" ]]; then
    echo "ERROR: failed to read database URL from secret dch-database-config in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
fi

echo "[4/7] PG database URL loaded"

# ---- 5. create PG secret + grant Flight SA read access ----
kubectl create secret generic "$E2E_PG_SECRET" \
    -n "$DCH_TENANT_ID" \
    --from-literal="url=${PG_INTERNAL_URL}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

kubectl create role e2e-flight-secret-read \
    -n "$DCH_TENANT_ID" \
    --verb=get --resource=secrets \
    --resource-name="$E2E_PG_SECRET" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

kubectl create rolebinding e2e-flight-secret-read-rb \
    -n "$DCH_TENANT_ID" \
    --role=e2e-flight-secret-read \
    --serviceaccount="${DCH_SERVICE_NAMESPACE}:${DCH_FLIGHT_SA}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

echo "[5/7] PG secret + Flight RBAC ready"

# ---- 6. seed test data via kubectl exec ----
PG_SVC_NAME=$(echo "$PG_INTERNAL_URL" | sed -n 's|.*@\([^:]*\):.*|\1|p' | cut -d. -f1)
PG_DB_NAME=$(echo "$PG_INTERNAL_URL" | sed -n 's|.*/\([^?]*\).*|\1|p')
PG_DB_USER=$(echo "$PG_INTERNAL_URL" | sed -n 's|.*://\([^:]*\):.*|\1|p')
PG_POD=$(kubectl get endpoints "$PG_SVC_NAME" -n "$DCH_SERVICE_NAMESPACE" \
    -o jsonpath='{.subsets[0].addresses[0].targetRef.name}' 2>/dev/null || true)
if [[ -z "$PG_POD" ]]; then
    echo "ERROR: no pod found behind service '$PG_SVC_NAME' in namespace '$DCH_SERVICE_NAMESPACE'" >&2; exit 1
fi

kubectl exec -i "$PG_POD" -n "$DCH_SERVICE_NAMESPACE" -- \
    psql -U "$PG_DB_USER" -d "$PG_DB_NAME" -v ON_ERROR_STOP=1 <<'SQL'
CREATE SCHEMA IF NOT EXISTS dch_e2e;
DROP TABLE IF EXISTS dch_e2e.cities;
CREATE TABLE dch_e2e.cities (
    id   SERIAL PRIMARY KEY,
    name TEXT    NOT NULL,
    country TEXT NOT NULL,
    population INTEGER NOT NULL
);
INSERT INTO dch_e2e.cities (name, country, population) VALUES
    ('Tokyo',  'Japan',          13960000),
    ('London', 'United Kingdom',  8982000),
    ('Paris',  'France',          2161000);
SQL
echo "[6/7] Test data seeded"

# ---- 7. generate SA token ----
if [[ -z "${DCH_AUTH_TOKEN:-}" ]]; then
    DCH_AUTH_TOKEN=$(kubectl create token "$E2E_SA_NAME" -n "$DCH_TENANT_ID" \
        --audience="$DCH_TOKEN_AUDIENCE" --duration=4h)
    echo "[7/7] SA token generated (4h TTL)"
else
    echo "[7/7] Using provided DCH_AUTH_TOKEN"
fi

# ---- write .env for pytest ----
cat > "$ENV_FILE" <<EOF
DCH_REST_URL=${DCH_REST_URL}
DCH_FLIGHT_URL=${DCH_FLIGHT_URL}
DCH_TENANT_ID=${DCH_TENANT_ID}
DCH_AUTH_TOKEN=${DCH_AUTH_TOKEN}
DCH_INSECURE=${DCH_INSECURE}
DCH_E2E_PG_SECRET=${E2E_PG_SECRET}
EOF

echo ""
echo "=== E2E Setup Complete ==="
echo "  .env written to: $ENV_FILE"
echo "  Run: make e2e-test"
