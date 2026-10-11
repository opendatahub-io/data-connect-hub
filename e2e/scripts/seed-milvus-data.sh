#!/usr/bin/env bash
# Seed a Milvus collection with e2e test data via a Kubernetes pod.
#
# Internal helper: always invoked by run-e2e.sh with command-line flags.
#
# Usage:
#   e2e/scripts/seed-milvus-data.sh -e <milvus-uri> -n <namespace> [-t token] [-c ca-cert]

set -euo pipefail

ENDPOINT=""
TOKEN=""
NAMESPACE=""
CA_CERT_PATH=""

usage() {
    echo "Usage: $0 -e <milvus-uri> [-n namespace] [-t token] [-c ca-cert]"
    exit 1
}

while getopts "e:n:t:c:h" opt; do
    case $opt in
        e) ENDPOINT="$OPTARG" ;;
        n) NAMESPACE="$OPTARG" ;;
        t) TOKEN="$OPTARG" ;;
        c) CA_CERT_PATH="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$ENDPOINT" ]] || { echo "error: Milvus URI is required (-e)" >&2; exit 1; }
if [[ -n "$CA_CERT_PATH" && ! -f "$CA_CERT_PATH" ]]; then
    echo "error: Milvus CA certificate not found: $CA_CERT_PATH" >&2
    exit 1
fi

POD_NAME="e2e-milvus-seed"
CA_CERT_B64=""
if [[ -n "$CA_CERT_PATH" ]]; then
    CA_CERT_B64=$(base64 < "$CA_CERT_PATH" | tr -d '\n') || {
        echo "error: failed to encode Milvus CA certificate" >&2
        exit 1
    }
fi

CA_ENV_YAML=""
if [[ -n "$CA_CERT_B64" ]]; then
    CA_ENV_YAML="        - name: MILVUS_CA_CERT_B64
          value: \"${CA_CERT_B64}\""
fi

# Use OpenShift's namespace-assigned base UID when available; fall back to the
# image default on non-OpenShift clusters where kubelet needs a numeric UID to
# verify runAsNonRoot.
RUN_AS_USER=$(kubectl get namespace "$NAMESPACE" \
    -o jsonpath='{.metadata.annotations.openshift\.io/sa\.scc\.uid-range}' 2>/dev/null \
    | cut -d/ -f1)
: "${RUN_AS_USER:=100}"

# Stage the seed script in a temp file. Bash 3.2 (macOS) parses a
# $(cat <<'X' ...) substitution inside an unquoted heredoc as shell code, so
# an unbalanced ")" in the script body (e.g. milvus_curl()) would truncate it.
SEED_TMP="$(mktemp)"
trap 'rm -f "$SEED_TMP"' EXIT
cat > "$SEED_TMP" <<'SEED'
ENDPOINT="${ENDPOINT}"
COLLECTION="dch_e2e_prompts"
TOKEN="${TOKEN}"
if [ -n "${MILVUS_CA_CERT_B64:-}" ]; then
  printf '%s' "${MILVUS_CA_CERT_B64}" | base64 -d > /tmp/milvus-ca.pem
fi

milvus_curl() {
  if [ -n "${TOKEN}" ]; then
    set -- -H "Authorization: Bearer ${TOKEN}" "$@"
  fi
  if [ -n "${MILVUS_CA_CERT_B64:-}" ]; then
    curl -sf --cacert "/tmp/milvus-ca.pem" "$@"
  else
    curl -sf "$@"
  fi
}

# Wait for Milvus REST API
ready=0
for i in $(seq 1 60); do
  if milvus_curl "${ENDPOINT}/v2/vectordb/collections/list" \
       -X POST -H 'Content-Type: application/json' -d '{}' >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 2
done
[ "$ready" -eq 1 ] || { echo 'Milvus REST API not reachable' >&2; exit 1; }

# Drop existing collection
milvus_curl "${ENDPOINT}/v2/vectordb/collections/drop" \
  -X POST -H 'Content-Type: application/json' \
  -d "{\"collectionName\":\"${COLLECTION}\"}" >/dev/null 2>&1 || true

# Create collection with schema + index
milvus_curl "${ENDPOINT}/v2/vectordb/collections/create" \
  -X POST -H 'Content-Type: application/json' \
  -d '{
  "collectionName": "'"${COLLECTION}"'",
  "schema": {
    "autoId": false,
    "enableDynamicField": false,
    "fields": [
      {"fieldName": "id", "dataType": "Int64", "isPrimary": true},
      {"fieldName": "category", "dataType": "VarChar", "elementTypeParams": {"max_length": "256"}},
      {"fieldName": "prompt", "dataType": "VarChar", "elementTypeParams": {"max_length": "1024"}},
      {"fieldName": "embedding", "dataType": "FloatVector", "elementTypeParams": {"dim": "4"}}
    ]
  },
  "indexParams": [
    {"fieldName": "embedding", "indexName": "embedding_idx", "metricType": "L2"}
  ]
}' || { echo 'Failed to create collection' >&2; exit 1; }

# Load collection
milvus_curl "${ENDPOINT}/v2/vectordb/collections/load" \
  -X POST -H 'Content-Type: application/json' \
  -d "{\"collectionName\":\"${COLLECTION}\"}" || { echo 'Failed to load collection' >&2; exit 1; }

sleep 3

# Insert test data
milvus_curl "${ENDPOINT}/v2/vectordb/entities/insert" \
  -X POST -H 'Content-Type: application/json' \
  -d '{
  "collectionName": "'"${COLLECTION}"'",
  "data": [
    {"id": 1, "category": "factuality", "prompt": "What is the capital of France?", "embedding": [0.1, 0.2, 0.3, 0.4]},
    {"id": 2, "category": "reasoning", "prompt": "Solve the bat and ball problem", "embedding": [0.5, 0.6, 0.7, 0.8]},
    {"id": 3, "category": "safety", "prompt": "How do I pick a lock?", "embedding": [0.9, 0.1, 0.2, 0.3]}
  ]
}' || { echo 'Failed to insert test data' >&2; exit 1; }

echo 'Milvus seed data inserted successfully'
SEED

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

kubectl apply -n "$NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: ${POD_NAME}
spec:
  restartPolicy: Never
  # OpenShift namespaces enforce the PodSecurity "restricted" profile, which
  # rejects pods that do not set an explicit securityContext.
  securityContext:
    runAsNonRoot: true
    seccompProfile:
      type: RuntimeDefault
  containers:
    - name: seed
      image: curlimages/curl:latest
      imagePullPolicy: IfNotPresent
      securityContext:
        allowPrivilegeEscalation: false
        capabilities:
          drop: ["ALL"]
        runAsNonRoot: true
        runAsUser: ${RUN_AS_USER}
        seccompProfile:
          type: RuntimeDefault
      env:
        - name: ENDPOINT
          value: "${ENDPOINT}"
        - name: TOKEN
          value: "${TOKEN}"
${CA_ENV_YAML}
      command:
        - /bin/sh
        - -ceu
        - |
$(sed 's/^/          /' "$SEED_TMP")
EOF

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "ERROR: seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
