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
: "${DCH_GATEWAY_ENDPOINT:?DCH_GATEWAY_ENDPOINT is required (set it in $CONFIG_FILE)}"

: "${DCH_TENANT_ID:?DCH_TENANT_ID is required (set it in $CONFIG_FILE)}"
: "${DCH_NO_ACCESS_NAMESPACE:?DCH_NO_ACCESS_NAMESPACE is required (set it in $CONFIG_FILE)}"
: "${DCH_FLIGHT_SA:?DCH_FLIGHT_SA is required (set it in $CONFIG_FILE)}"
DCH_TOKEN_AUDIENCE="${DCH_TOKEN_AUDIENCE:-}"
: "${DCH_INSECURE:?DCH_INSECURE is required (set it in $CONFIG_FILE)}"
: "${DCH_POSTGRES_IMAGE:?DCH_POSTGRES_IMAGE is required (set it in $CONFIG_FILE)}"

DCH_TENANT_PG_URL="${DCH_TENANT_PG_URL:-}"
DCH_TENANT_MILVUS_HOST="${DCH_TENANT_MILVUS_HOST:-}"
DCH_TENANT_MILVUS_PORT="${DCH_TENANT_MILVUS_PORT:-19530}"
DCH_TENANT_ES_URI="${DCH_TENANT_ES_URI:-}"
DCH_TENANT_ES_NAMESPACE="${DCH_TENANT_ES_NAMESPACE:-$DCH_TENANT_ID}"
DCH_TENANT_ES_USERNAME="${DCH_TENANT_ES_USERNAME:-}"
DCH_TENANT_ES_PASSWORD="${DCH_TENANT_ES_PASSWORD:-}"
DCH_TENANT_ES_CA_CERT="${DCH_TENANT_ES_CA_CERT:-}"
DCH_TENANT_NEO4J_URI="${DCH_TENANT_NEO4J_URI:-}"
DCH_TENANT_NEO4J_ADMIN_PASSWORD="${DCH_TENANT_NEO4J_ADMIN_PASSWORD:-}"
DCH_TENANT_NEO4J_USERNAME="${DCH_TENANT_NEO4J_USERNAME:-dch_reader}"
DCH_TENANT_NEO4J_PASSWORD="${DCH_TENANT_NEO4J_PASSWORD:-dch_readonly}"
DCH_TENANT_URI="${DCH_TENANT_URI:-}"

E2E_SA_NAME="e2e-user"
E2E_DENIED_SA_NAME="e2e-denied-user"
PG_SECRET="e2e-pg-creds"
S3_SECRET="e2e-s3-creds"
MILVUS_SECRET="e2e-milvus-creds"
ES_SECRET="e2e-es-creds"
ES_APIKEY_SECRET="e2e-es-apikey-creds"
NEO4J_SECRET="e2e-neo4j-creds"
URI_SECRET="e2e-uri-creds"
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
    E2E_PG_ENABLED="false"
    if [[ -n "$DCH_TENANT_PG_URL" ]]; then
        PG_INTERNAL_URL="$DCH_TENANT_PG_URL"
        kubectl create secret generic "$PG_SECRET" \
            -n "$DCH_TENANT_ID" \
            --from-literal="URI=${PG_INTERNAL_URL}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_PG_ENABLED="true"
    fi
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
    if [[ -n "$DCH_TENANT_MILVUS_HOST" ]]; then
        local -a args=(
            --from-literal="MILVUS_HOST=${DCH_TENANT_MILVUS_HOST}"
            --from-literal="MILVUS_PORT=${DCH_TENANT_MILVUS_PORT}"
        )
        [[ -n "${DCH_TENANT_MILVUS_TOKEN:-}" ]] && args+=(--from-literal="MILVUS_TOKEN=${DCH_TENANT_MILVUS_TOKEN}")
        [[ -n "${DCH_TENANT_MILVUS_DATABASE:-}" ]] && args+=(--from-literal="MILVUS_DATABASE=${DCH_TENANT_MILVUS_DATABASE}")
        kubectl create secret generic "$MILVUS_SECRET" \
            -n "$DCH_TENANT_ID" \
            "${args[@]}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_MILVUS_ENABLED="true"
    fi
}

fetch_es_ca_cert() {
    local es_namespace="$DCH_TENANT_ES_NAMESPACE"
    kubectl get secret elasticsearch-master-certs -n "$es_namespace" \
        -o jsonpath='{.data.ca\.crt}' 2>/dev/null | base64 -d 2>/dev/null || true
}

setup_es_secret() {
    E2E_ES_ENABLED="false"
    if [[ -n "$DCH_TENANT_ES_URI" ]]; then
        local -a args=(--from-literal="ES_HOST=${DCH_TENANT_ES_URI}")
        [[ -n "$DCH_TENANT_ES_USERNAME" ]] && args+=(--from-literal="ES_USERNAME=${DCH_TENANT_ES_USERNAME}")
        [[ -n "$DCH_TENANT_ES_PASSWORD" ]] && args+=(--from-literal="ES_PASSWORD=${DCH_TENANT_ES_PASSWORD}")

        local ca_cert="$DCH_TENANT_ES_CA_CERT"
        if [[ -z "$ca_cert" ]]; then
            ca_cert=$(fetch_es_ca_cert)
        fi
        [[ -n "$ca_cert" ]] && args+=(--from-literal="ES_CA_CERT=${ca_cert}")

        kubectl create secret generic "$ES_SECRET" \
            -n "$DCH_TENANT_ID" \
            "${args[@]}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_ES_ENABLED="true"
    fi
}

setup_es_apikey_secret() {
    E2E_ES_APIKEY_ENABLED="false"
    [[ "$E2E_ES_ENABLED" == "true" ]] || return 0
    [[ -n "$DCH_TENANT_ES_USERNAME" && -n "$DCH_TENANT_ES_PASSWORD" ]] || return 0

    local es_namespace="$DCH_TENANT_ES_NAMESPACE"
    local es_pod
    es_pod=$(kubectl get pods -n "$es_namespace" -l app=elasticsearch-master \
        -o jsonpath='{.items[0].metadata.name}' 2>/dev/null) || return 0
    [[ -n "$es_pod" ]] || return 0

    local api_key_json
    api_key_json=$(kubectl exec "$es_pod" -n "$es_namespace" -- \
        curl -ksf -u "${DCH_TENANT_ES_USERNAME}:${DCH_TENANT_ES_PASSWORD}" \
        -X POST "https://localhost:9200/_security/api_key" \
        -H "Content-Type: application/json" \
        -d '{"name":"e2e-test-key"}' 2>/dev/null) || return 0

    local encoded_api_key
    encoded_api_key=$(echo "$api_key_json" | python3 -c "import sys,json; print(json.load(sys.stdin)['encoded'])" 2>/dev/null) || return 0

    local -a args=(
        --from-literal="ES_HOST=${DCH_TENANT_ES_URI}"
        --from-literal="ES_API_KEY=${encoded_api_key}"
    )
    local ca_cert="$DCH_TENANT_ES_CA_CERT"
    if [[ -z "$ca_cert" ]]; then
        ca_cert=$(fetch_es_ca_cert)
    fi
    [[ -n "$ca_cert" ]] && args+=(--from-literal="ES_CA_CERT=${ca_cert}")

    kubectl create secret generic "$ES_APIKEY_SECRET" \
        -n "$DCH_TENANT_ID" \
        "${args[@]}" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null
    E2E_ES_APIKEY_ENABLED="true"
}

setup_neo4j_secret() {
    E2E_NEO4J_ENABLED="false"
    if [[ -n "$DCH_TENANT_NEO4J_URI" ]]; then
        local -a args=(
            --from-literal="NEO4J_URI=${DCH_TENANT_NEO4J_URI}"
            --from-literal="NEO4J_USERNAME=${DCH_TENANT_NEO4J_USERNAME}"
            --from-literal="NEO4J_PASSWORD=${DCH_TENANT_NEO4J_PASSWORD}"
        )
        [[ -n "${DCH_TENANT_NEO4J_DATABASE:-}" ]] && args+=(--from-literal="NEO4J_DATABASE=${DCH_TENANT_NEO4J_DATABASE}")
        kubectl create secret generic "$NEO4J_SECRET" \
            -n "$DCH_TENANT_ID" \
            "${args[@]}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_NEO4J_ENABLED="true"
    fi
}

setup_uri_server_and_secret() {
    E2E_URI_ENABLED="false"
    if [[ -n "$DCH_TENANT_URI" ]]; then
        # Use externally provided URI
        kubectl create secret generic "$URI_SECRET" \
            -n "$DCH_TENANT_ID" \
            --from-literal="URI=${DCH_TENANT_URI}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_URI_ENABLED="true"
    elif [[ "${DCH_URI_DEPLOY_SERVER:-true}" == "true" ]]; then
        # Deploy a test HTTP server and create the secret automatically
        bash "$(dirname "$0")/scripts/seed-uri-data.sh" -n "$DCH_TENANT_ID"
        local uri="http://e2e-uri-server.${DCH_TENANT_ID}.svc:8080"
        kubectl create secret generic "$URI_SECRET" \
            -n "$DCH_TENANT_ID" \
            --from-literal="URI=${uri}" \
            --dry-run=client -o yaml | kubectl apply -f - >/dev/null
        E2E_URI_ENABLED="true"
    fi
}

setup_flight_secret_rbac() {
    local -a secret_names=()
    [[ "$E2E_PG_ENABLED" == "true" ]] && secret_names+=("--resource-name=$PG_SECRET")
    [[ "$E2E_S3_ENABLED" == "true" ]] && secret_names+=("--resource-name=$S3_SECRET")
    [[ "$E2E_MILVUS_ENABLED" == "true" ]] && secret_names+=("--resource-name=$MILVUS_SECRET")
    [[ "$E2E_ES_ENABLED" == "true" ]] && secret_names+=("--resource-name=$ES_SECRET")
    [[ "$E2E_ES_APIKEY_ENABLED" == "true" ]] && secret_names+=("--resource-name=$ES_APIKEY_SECRET")
    [[ "$E2E_NEO4J_ENABLED" == "true" ]] && secret_names+=("--resource-name=$NEO4J_SECRET")
    [[ "$E2E_URI_ENABLED" == "true" ]] && secret_names+=("--resource-name=$URI_SECRET")

    if [[ ${#secret_names[@]} -eq 0 ]]; then
        return 0
    fi

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
    [[ "$E2E_PG_ENABLED" == "true" ]] || return 0
    bash "$(dirname "$0")/scripts/seed-postgresql-data.sh" \
        -u "$PG_INTERNAL_URL" -n "$DCH_TENANT_ID" -i "$DCH_POSTGRES_IMAGE"
}

seed_s3_data() {
    [[ "$E2E_S3_ENABLED" == "true" ]] || return 0
    [[ "${DCH_S3_SEED_DATASET:-}" == "true" ]] || return 0
    # Require a digest-pinned MinIO client image (see CWE-494 in PR review).
    local mc_image="${DCH_MINIO_MC_IMAGE:-}"
    [[ -n "$mc_image" ]] || {
        echo "ERROR: set DCH_MINIO_MC_IMAGE to a digest-pinned image (e.g. minio/mc@sha256:<digest>) in $CONFIG_FILE" >&2
        exit 1
    }
    bash "$(dirname "$0")/scripts/seed-s3-data.sh" \
        -e "$AWS_S3_ENDPOINT" -n "$DCH_TENANT_ID" -b "$AWS_S3_BUCKET" \
        -A "$AWS_ACCESS_KEY_ID" -S "$AWS_SECRET_ACCESS_KEY" \
        -i "$mc_image"
}

seed_milvus_data() {
    [[ "$E2E_MILVUS_ENABLED" == "true" ]] || return 0
    local milvus_uri="http://${DCH_TENANT_MILVUS_HOST}:${DCH_TENANT_MILVUS_PORT}"
    bash "$(dirname "$0")/scripts/seed-milvus-data.sh" \
        -e "$milvus_uri" -n "$DCH_TENANT_ID"
}

seed_neo4j_data() {
    [[ "$E2E_NEO4J_ENABLED" == "true" ]] || return 0
    [[ -n "$DCH_TENANT_NEO4J_ADMIN_PASSWORD" ]] || {
        echo "ERROR: set DCH_TENANT_NEO4J_ADMIN_PASSWORD in $CONFIG_FILE to seed Neo4j" >&2
        exit 1
    }
    bash "$(dirname "$0")/scripts/seed-neo4j-data.sh" \
        -u "$DCH_TENANT_NEO4J_URI" -n "$DCH_TENANT_ID" \
        -a "$DCH_TENANT_NEO4J_ADMIN_PASSWORD" \
        --user "$DCH_TENANT_NEO4J_USERNAME" \
        --pass "$DCH_TENANT_NEO4J_PASSWORD"
}

seed_es_data() {
    [[ "$E2E_ES_ENABLED" == "true" ]] || return 0
    local -a args=(-e "$DCH_TENANT_ES_URI" -n "$DCH_TENANT_ID")
    [[ -n "$DCH_TENANT_ES_PASSWORD" ]] && args+=(-p "$DCH_TENANT_ES_PASSWORD")
    bash "$(dirname "$0")/scripts/seed-elasticsearch-data.sh" "${args[@]}"
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
DCH_GATEWAY_ENDPOINT=${DCH_GATEWAY_ENDPOINT}
DCH_TENANT_ID=${DCH_TENANT_ID}
DCH_NO_ACCESS_NAMESPACE=${DCH_NO_ACCESS_NAMESPACE}
DCH_AUTH_TOKEN=${DCH_AUTH_TOKEN}
DCH_DENIED_AUTH_TOKEN=${DCH_DENIED_AUTH_TOKEN}
DCH_INSECURE=${DCH_INSECURE}
EOF

    if [[ "$E2E_PG_ENABLED" == "true" ]]; then
        echo "DCH_PG_SECRET=${PG_SECRET}" >> "$ENV_FILE"
    fi

    [[ -n "${DCH_FLIGHT_METRICS_URL:-}" ]] && \
        echo "DCH_FLIGHT_METRICS_URL=${DCH_FLIGHT_METRICS_URL}" >> "$ENV_FILE"

    if [[ "$E2E_S3_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<EOF
DCH_S3_SECRET=${S3_SECRET}
DCH_S3_CSV_QUERY=datasets/dch-test-prompts.csv
DCH_S3_PARQUET_QUERY=datasets/dch-test-prompts.parquet
DCH_S3_JSONL_QUERY=datasets/dch-test-prompts.jsonl
DCH_S3_BINARY_PATH=datasets/dch-test-binary.bin
EOF
    fi

    if [[ "$E2E_MILVUS_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<'MILVUS_EOF'
DCH_MILVUS_SECRET=e2e-milvus-creds
MILVUS_EOF
    fi

    if [[ "$E2E_ES_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<'ES_EOF'
DCH_ES_SECRET=e2e-es-creds
ES_EOF
    fi

    if [[ "$E2E_ES_APIKEY_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<'ES_APIKEY_EOF'
DCH_ES_APIKEY_SECRET=e2e-es-apikey-creds
ES_APIKEY_EOF
    fi

    if [[ "$E2E_NEO4J_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<'NEO4J_EOF'
DCH_NEO4J_SECRET=e2e-neo4j-creds
NEO4J_EOF
    fi

    if [[ "$E2E_URI_ENABLED" == "true" ]]; then
        cat >> "$ENV_FILE" <<'URI_EOF'
DCH_URI_SECRET=e2e-uri-creds
URI_EOF
    fi
}

# ===================================================================
# Main
# ===================================================================

echo "=== E2E Setup ==="

# 1. Install dependencies
VENV_DIR="$SCRIPT_DIR/.venv"
if [[ ! -d "$VENV_DIR" ]]; then
    python3 -m venv "$VENV_DIR"
fi
VENV_PYTHON="$VENV_DIR/bin/python3"
VENV_PYTEST="$VENV_DIR/bin/pytest"
if [[ ! -x "$VENV_PYTEST" ]]; then
    "$VENV_PYTHON" -m pip install --quiet \
        -e "$REPO_ROOT/sdk/python[flight]" \
        -e "$SCRIPT_DIR"
fi
echo "[1/11] Dependencies ready"

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
echo "[2/11] Namespaces ready"

setup_service_accounts
echo "[3/11] Service accounts ready"

setup_sa_rbac
echo "[4/11] SA RBAC ready"

# 4. Credential secrets
setup_pg_secret
setup_s3_secret
setup_milvus_secret
setup_es_secret
setup_es_apikey_secret
setup_neo4j_secret
setup_uri_server_and_secret
setup_flight_secret_rbac

SECRETS_MSG=""
[[ "$E2E_PG_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG:+$SECRETS_MSG + }PG"
[[ "$E2E_S3_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG:+$SECRETS_MSG + }S3"
[[ "$E2E_MILVUS_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG:+$SECRETS_MSG + }Milvus"
[[ "$E2E_ES_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG:+$SECRETS_MSG + }Elasticsearch"
[[ "$E2E_NEO4J_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG:+$SECRETS_MSG + }Neo4j"
[[ "$E2E_URI_ENABLED" == "true" ]] && SECRETS_MSG="${SECRETS_MSG:+$SECRETS_MSG + }URI"
echo "[5/11] ${SECRETS_MSG:-none} secrets + Flight RBAC ready"

# 5. Seed test data
seed_pg_data
if [[ "$E2E_PG_ENABLED" == "true" ]]; then
    echo "[6/11] PG test data seeded"
else
    echo "[6/11] PG seed skipped (DCH_TENANT_PG_URL not set)"
fi

seed_s3_data
if [[ "$E2E_S3_ENABLED" == "true" && "${DCH_S3_SEED_DATASET:-}" == "true" ]]; then
    echo "[7/11] S3 test data seeded"
else
    echo "[7/11] S3 seed skipped"
fi

seed_milvus_data
if [[ "$E2E_MILVUS_ENABLED" == "true" ]]; then
    echo "[8/11] Milvus test data seeded"
else
    echo "[8/11] Milvus seed skipped (DCH_TENANT_MILVUS_HOST not set)"
fi

seed_es_data
if [[ "$E2E_ES_ENABLED" == "true" ]]; then
    echo "[9/11] Elasticsearch test data seeded"
else
    echo "[9/11] Elasticsearch seed skipped (DCH_TENANT_ES_URI not set)"
fi

seed_neo4j_data
if [[ "$E2E_NEO4J_ENABLED" == "true" ]]; then
    echo "[10/11] Neo4j test data seeded"
else
    echo "[10/11] Neo4j seed skipped (DCH_TENANT_NEO4J_URI not set)"
fi

# 6. Auth tokens
generate_tokens
echo "[11/11] Auth tokens + .env ready"

# 7. Write .env
write_env_file

echo ""
echo "=== E2E Setup Complete ==="

# -------------------------------------------------------------------
# Run tests
# -------------------------------------------------------------------

echo ""
echo "=== Running E2E Tests ==="
cd "$SCRIPT_DIR"
exec "$VENV_PYTEST" tests/ -v "$@"
