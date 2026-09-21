#!/usr/bin/env bash
# Stage 3: Run e2e tests and dump service logs.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

e2e_env_file="${CI_TEMP_DIR}/e2e-ci.env"

# ===================================================================
# Generate e2e env config
# ===================================================================

echo "=== Generating e2e config (connectors: ${CI_ENABLED_CONNECTORS}) ==="

cat > "$e2e_env_file" <<EOF
#### Required ####
DCH_SERVICE_NAMESPACE=${CI_SVC_NAMESPACE}
DCH_GATEWAY_ENDPOINT=127.0.0.1:${CI_GATEWAY_LOCAL_PORT}
DCH_TENANT_ID=${CI_TENANT_NAMESPACE}
DCH_FLIGHT_SA=${CI_FLIGHT_SA_NAME}
DCH_REST_SA=${CI_REST_SA_NAME}

#### DCH Service ####
DCH_INSECURE=true
DCH_TOKEN_AUDIENCE=${CI_SA_TOKEN_AUDIENCE}
DCH_NO_ACCESS_NAMESPACE=${CI_NO_ACCESS_NAMESPACE}
DCH_FLIGHT_METRICS_URL=http://127.0.0.1:${CI_FLIGHT_METRICS_LOCAL_PORT}
EOF


if has_connector postgres; then
    tenant_pg_url="postgresql://${CI_TENANT_PG_USER}:${CI_TENANT_PG_PASSWORD}@${CI_TENANT_PG_HOST}.${CI_TENANT_NAMESPACE}.svc:5432/${CI_TENANT_PG_DATABASE}"
    tenant_pg_ca_cert=""
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        tenant_pg_url="${tenant_pg_url}?sslmode=verify-ca"
        tenant_pg_ca_cert="${CI_TEMP_DIR}/dch-tenant-pg-ca.crt"
        kubectl get secret "${CI_TENANT_PG_HOST}-tls" -n "$CI_TENANT_NAMESPACE" \
            -o jsonpath='{.data.ca\.crt}' | base64 -d > "$tenant_pg_ca_cert"
    fi
    cat >> "$e2e_env_file" <<EOF
#### PostgreSQL ####
DCH_TENANT_PG_URL=${tenant_pg_url}
DCH_TENANT_PG_CA_CERT=${tenant_pg_ca_cert}
EOF
fi

if has_connector s3; then
    tenant_s3_endpoint="http://${CI_TENANT_MINIO_RELEASE}.${CI_TENANT_NAMESPACE}.svc:9000"
    tenant_s3_ca_cert=""
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        tenant_s3_endpoint="https://${CI_TENANT_MINIO_RELEASE}.${CI_TENANT_NAMESPACE}.svc:9000"
        tenant_s3_ca_cert="${CI_TEMP_DIR}/dch-tenant-minio-ca.crt"
        kubectl get secret "${CI_TENANT_MINIO_RELEASE}-tls" -n "$CI_TENANT_NAMESPACE" \
            -o jsonpath='{.data.ca\.crt}' | base64 -d > "$tenant_s3_ca_cert" || {
            echo "ERROR: failed to retrieve MinIO CA certificate" >&2
            exit 1
        }
        [[ -s "$tenant_s3_ca_cert" ]] || {
            echo "ERROR: MinIO CA certificate is empty" >&2
            exit 1
        }
    fi
    cat >> "$e2e_env_file" <<EOF
#### AWS S3 ####
AWS_ACCESS_KEY_ID=${CI_TENANT_MINIO_ROOT_USER}
AWS_SECRET_ACCESS_KEY=${CI_TENANT_MINIO_ROOT_PASSWORD}
AWS_S3_BUCKET=${CI_TENANT_MINIO_BUCKET}
AWS_DEFAULT_REGION=us-east-1
AWS_S3_ENDPOINT=${tenant_s3_endpoint}
AWS_S3_CA_CERT=${tenant_s3_ca_cert}
DCH_S3_SEED_DATASET=true
EOF
fi


if has_connector milvus; then
    tenant_milvus_uri="http://milvus.${CI_TENANT_NAMESPACE}.svc:19530"
    tenant_milvus_ca_cert=""
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        tenant_milvus_uri="https://milvus.${CI_TENANT_NAMESPACE}.svc.cluster.local:8080"
        tenant_milvus_ca_cert="${CI_TEMP_DIR}/dch-tenant-milvus-ca.pem"
        kubectl get secret milvus-milvus-tls -n "$CI_TENANT_NAMESPACE" \
            -o jsonpath='{.data.ca\.pem}' | base64 -d > "$tenant_milvus_ca_cert" || {
            echo "ERROR: failed to retrieve Milvus CA certificate" >&2
            exit 1
        }
        [[ -s "$tenant_milvus_ca_cert" ]] || {
            echo "ERROR: Milvus CA certificate is empty" >&2
            exit 1
        }
    fi
    cat >> "$e2e_env_file" <<EOF
#### Milvus ####
DCH_TENANT_MILVUS_URI=${tenant_milvus_uri}
DCH_TENANT_MILVUS_CA_CERT=${tenant_milvus_ca_cert}
EOF
fi

if has_connector elasticsearch; then
    tenant_es_scheme="http"
    tenant_es_ca_cert=""
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        tenant_es_scheme="https"
        tenant_es_ca_secret=$(kubectl get secret \
            -n "$CI_TENANT_NAMESPACE" \
            -l "release=${CI_TENANT_ES_HELM_RELEASE},chart=elasticsearch" \
            -o jsonpath='{.items[?(@.type=="kubernetes.io/tls")].metadata.name}') || {
            echo "ERROR: failed to find Elasticsearch CA secret" >&2
            exit 1
        }
        [[ -n "$tenant_es_ca_secret" ]] || {
            echo "ERROR: Elasticsearch CA secret is missing" >&2
            exit 1
        }
        tenant_es_ca_cert="${CI_TEMP_DIR}/dch-tenant-es-ca.crt"
        kubectl get secret "$tenant_es_ca_secret" \
            -n "$CI_TENANT_NAMESPACE" \
            -o jsonpath='{.data.ca\.crt}' | base64 -d > "$tenant_es_ca_cert" || {
            echo "ERROR: failed to retrieve Elasticsearch CA certificate" >&2
            exit 1
        }
        [[ -s "$tenant_es_ca_cert" ]] || {
            echo "ERROR: Elasticsearch CA certificate is empty" >&2
            exit 1
        }
    fi
    cat >> "$e2e_env_file" <<EOF
#### Elasticsearch ####
DCH_TENANT_ES_URI=${tenant_es_scheme}://${CI_TENANT_ES_HELM_RELEASE}-master.${CI_TENANT_NAMESPACE}.svc:9200
DCH_TENANT_ES_NAMESPACE=${CI_TENANT_NAMESPACE}
DCH_TENANT_ES_USERNAME=elastic
DCH_TENANT_ES_PASSWORD=${CI_TENANT_ES_PASSWORD}
DCH_TENANT_ES_CA_CERT=${tenant_es_ca_cert}
EOF
fi

if has_connector neo4j; then
    tenant_neo4j_uri="bolt://${CI_TENANT_NEO4J_HELM_RELEASE}.${CI_TENANT_NAMESPACE}.svc:7687"
    tenant_neo4j_ca_cert=""
    if [[ "$CI_SSL_ENABLED" == "true" ]]; then
        tenant_neo4j_uri="neo4j+s://${CI_TENANT_NEO4J_HELM_RELEASE}.${CI_TENANT_NAMESPACE}.svc:7687"
        tenant_neo4j_ca_cert="${CI_TEMP_DIR}/dch-tenant-neo4j-ca.crt"
        kubectl get secret "${CI_TENANT_NEO4J_HELM_RELEASE}-tls-ca" -n "$CI_TENANT_NAMESPACE" \
            -o jsonpath='{.data.ca\.crt}' | base64 -d > "$tenant_neo4j_ca_cert" || {
            echo "ERROR: failed to retrieve Neo4j CA certificate" >&2
            exit 1
        }
        [[ -s "$tenant_neo4j_ca_cert" ]] || {
            echo "ERROR: Neo4j CA certificate is empty" >&2
            exit 1
        }
    fi
    cat >> "$e2e_env_file" <<EOF
#### Neo4j ####
DCH_TENANT_NEO4J_URI=${tenant_neo4j_uri}
DCH_TENANT_NEO4J_ADMIN_PASSWORD=${CI_TENANT_NEO4J_ADMIN_PASSWORD}
DCH_TENANT_NEO4J_USERNAME=${CI_TENANT_NEO4J_USERNAME}
DCH_TENANT_NEO4J_PASSWORD=${CI_TENANT_NEO4J_PASSWORD}
DCH_TENANT_NEO4J_CA_CERT=${tenant_neo4j_ca_cert}
DCH_TENANT_NEO4J_DATABASE=neo4j
EOF
fi

if has_connector uri; then
    cat >> "$e2e_env_file" <<EOF
#### URI ####
DCH_TENANT_URI_DEPLOY_SERVER=true
EOF
fi

echo "E2E config:"
cat "$e2e_env_file"

# ===================================================================
# Run tests
# ===================================================================

echo ""
echo "=== Running E2E Tests ==="
bash "$CI_REPO_ROOT/e2e/run-e2e.sh" "$e2e_env_file" -s
