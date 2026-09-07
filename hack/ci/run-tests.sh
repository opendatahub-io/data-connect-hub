#!/usr/bin/env bash
# Stage 3: Run e2e tests and dump service logs.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

# ===================================================================
# Generate e2e env config
# ===================================================================

echo "=== Generating e2e config (datasources: ${E2E_DATASOURCES}) ==="

ENV_FILE="${TEMP_DIR}/e2e-ci.env"
cat > "$ENV_FILE" <<EOF
#### Common ####
DCH_SERVICE_NAMESPACE=${SVC_NAMESPACE}
DCH_GATEWAY_ENDPOINT=127.0.0.1:${GATEWAY_LOCAL_PORT}
DCH_FLIGHT_METRICS_URL=http://127.0.0.1:${FLIGHT_METRICS_PORT}
DCH_TENANT_ID=${TENANT_NAMESPACE}
DCH_NO_ACCESS_NAMESPACE=${NO_ACCESS_NAMESPACE}
DCH_FLIGHT_SA=${FLIGHT_SA_NAME}
DCH_REST_SA=${REST_SA_NAME}
DCH_TOKEN_AUDIENCE=${SA_TOKEN_AUDIENCE}
DCH_INSECURE=true
EOF

if has_datasource s3; then
    MINIO_MC_DIGEST=$(docker inspect --format='{{index .RepoDigests 0}}' "$MINIO_MC_IMAGE")
    cat >> "$ENV_FILE" <<EOF
#### S3 (MinIO) ####
DCH_S3_SEED_DATASET=true
AWS_S3_BUCKET=${MINIO_BUCKET}
AWS_DEFAULT_REGION=us-east-1
AWS_S3_ENDPOINT=http://${MINIO_RELEASE}.${TENANT_NAMESPACE}.svc:9000
AWS_ACCESS_KEY_ID=${MINIO_ROOT_USER}
AWS_SECRET_ACCESS_KEY=${MINIO_ROOT_PASSWORD}
DCH_MINIO_MC_IMAGE=${MINIO_MC_DIGEST}
EOF
fi

if has_datasource postgres; then
    TENANT_PG_URL="postgresql://dch_tenant_user:dch_tenant_password@dch-tenant-postgres.${TENANT_NAMESPACE}.svc:5432/dch_tenant_db"
    TENANT_PG_CA_CERT=""
    if [[ "$POSTGRES_SSL_MODE" != "disable" ]]; then
        TENANT_PG_URL="${TENANT_PG_URL}?sslmode=${POSTGRES_SSL_MODE}"
        if [[ "$POSTGRES_SSL_MODE" == "verify-ca" || "$POSTGRES_SSL_MODE" == "verify-full" ]]; then
            TENANT_PG_CA_CERT="${TEMP_DIR}/dch-tenant-ca.crt"
            kubectl get secret dch-tenant-postgres-tls -n "$TENANT_NAMESPACE" \
                -o jsonpath='{.data.ca\.crt}' | base64 -d > "$TENANT_PG_CA_CERT"
        fi
    fi
    cat >> "$ENV_FILE" <<EOF
#### Postgres ####
DCH_TENANT_PG_URL=${TENANT_PG_URL}
DCH_TENANT_PG_CA_CERT=${TENANT_PG_CA_CERT}
DCH_POSTGRES_IMAGE=${POSTGRES_IMAGE}
EOF
fi

if has_datasource milvus; then
    cat >> "$ENV_FILE" <<EOF
#### Milvus ####
DCH_TENANT_MILVUS_URI=http://milvus.${TENANT_NAMESPACE}.svc:19530
EOF
fi

if has_datasource elasticsearch; then
    cat >> "$ENV_FILE" <<EOF
#### Elasticsearch ####
DCH_TENANT_ES_URI=https://${ES_HELM_RELEASE}-master.${TENANT_NAMESPACE}.svc:9200
DCH_TENANT_ES_NAMESPACE=${TENANT_NAMESPACE}
DCH_TENANT_ES_USERNAME=elastic
DCH_TENANT_ES_PASSWORD=${ES_PASSWORD}
EOF
fi

if has_datasource neo4j; then
    cat >> "$ENV_FILE" <<EOF
#### Neo4j ####
DCH_TENANT_NEO4J_URI=bolt://${NEO4J_HELM_RELEASE}.${TENANT_NAMESPACE}.svc:7687
DCH_TENANT_NEO4J_ADMIN_PASSWORD=${NEO4J_ADMIN_PASSWORD}
DCH_TENANT_NEO4J_USERNAME=${NEO4J_USERNAME}
DCH_TENANT_NEO4J_PASSWORD=${NEO4J_PASSWORD}
EOF
fi

if has_datasource uri; then
    cat >> "$ENV_FILE" <<EOF
#### URI ####
DCH_URI_DEPLOY_SERVER=true
EOF
fi

echo "E2E config:"
cat "$ENV_FILE"

# ===================================================================
# Run tests
# ===================================================================

echo ""
echo "=== Running E2E Tests ==="
bash "$REPO_ROOT/e2e/run-e2e.sh" "$ENV_FILE" -s
