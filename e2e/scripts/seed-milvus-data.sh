#!/usr/bin/env bash
# Seed a Milvus collection with e2e test data via a Kubernetes pod.
#
# Usage:
#   e2e/scripts/seed-milvus-data.sh -e http://milvus:19530 -n <namespace>
#
# Environment overrides (command-line flags take precedence):
#   DCH_MILVUS_URI          Milvus REST endpoint     (required)
#   DCH_MILVUS_TOKEN        Auth token               (optional)
#   DCH_SERVICE_NAMESPACE   Namespace for seed pod   (default: dch)

set -euo pipefail

ENDPOINT="${DCH_MILVUS_URI:-}"
TOKEN="${DCH_MILVUS_TOKEN:-}"
NAMESPACE="${DCH_SERVICE_NAMESPACE:-dch}"
COLLECTION="dch_e2e_prompts"

usage() {
    echo "Usage: $0 -e <milvus-uri> [-n namespace] [-t token]"
    exit 1
}

while getopts "e:n:t:h" opt; do
    case $opt in
        e) ENDPOINT="$OPTARG" ;;
        n) NAMESPACE="$OPTARG" ;;
        t) TOKEN="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$ENDPOINT" ]] || { echo "error: Milvus URI is required (-e or DCH_MILVUS_URI)" >&2; exit 1; }

POD_NAME="e2e-milvus-seed"

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
kubectl run "$POD_NAME" -n "$NAMESPACE" \
    --image="curlimages/curl:latest" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    --command -- /bin/sh -ceu "
ENDPOINT='${ENDPOINT}'
COLLECTION='${COLLECTION}'
TOKEN='${TOKEN}'

AUTH=''
[ -n \"\${TOKEN}\" ] && AUTH=\"Authorization: Bearer \${TOKEN}\"

# Wait for Milvus REST API
ready=0
for i in \$(seq 1 60); do
  if curl -sf \"\${ENDPOINT}/v2/vectordb/collections/list\" \
       -X POST -H 'Content-Type: application/json' \${AUTH:+-H \"\${AUTH}\"} -d '{}' >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 2
done
[ \"\$ready\" -eq 1 ] || { echo 'Milvus REST API not reachable' >&2; exit 1; }

# Drop existing collection
curl -sf \"\${ENDPOINT}/v2/vectordb/collections/drop\" \
  -X POST -H 'Content-Type: application/json' \${AUTH:+-H \"\${AUTH}\"} \
  -d \"{\\\"collectionName\\\":\\\"\${COLLECTION}\\\"}\" >/dev/null 2>&1 || true

# Create collection with schema + index
curl -sf \"\${ENDPOINT}/v2/vectordb/collections/create\" \
  -X POST -H 'Content-Type: application/json' \${AUTH:+-H \"\${AUTH}\"} \
  -d '{
  \"collectionName\": \"'\"\${COLLECTION}\"'\",
  \"schema\": {
    \"autoId\": false,
    \"enableDynamicField\": false,
    \"fields\": [
      {\"fieldName\": \"id\", \"dataType\": \"Int64\", \"isPrimary\": true},
      {\"fieldName\": \"category\", \"dataType\": \"VarChar\", \"elementTypeParams\": {\"max_length\": \"256\"}},
      {\"fieldName\": \"prompt\", \"dataType\": \"VarChar\", \"elementTypeParams\": {\"max_length\": \"1024\"}},
      {\"fieldName\": \"embedding\", \"dataType\": \"FloatVector\", \"elementTypeParams\": {\"dim\": \"4\"}}
    ]
  },
  \"indexParams\": [
    {\"fieldName\": \"embedding\", \"indexName\": \"embedding_idx\", \"metricType\": \"L2\"}
  ]
}' || { echo 'Failed to create collection' >&2; exit 1; }

# Load collection
curl -sf \"\${ENDPOINT}/v2/vectordb/collections/load\" \
  -X POST -H 'Content-Type: application/json' \${AUTH:+-H \"\${AUTH}\"} \
  -d \"{\\\"collectionName\\\":\\\"\${COLLECTION}\\\"}\" || { echo 'Failed to load collection' >&2; exit 1; }

sleep 3

# Insert test data
curl -sf \"\${ENDPOINT}/v2/vectordb/entities/insert\" \
  -X POST -H 'Content-Type: application/json' \${AUTH:+-H \"\${AUTH}\"} \
  -d '{
  \"collectionName\": \"'\"\${COLLECTION}\"'\",
  \"data\": [
    {\"id\": 1, \"category\": \"factuality\", \"prompt\": \"What is the capital of France?\", \"embedding\": [0.1, 0.2, 0.3, 0.4]},
    {\"id\": 2, \"category\": \"reasoning\", \"prompt\": \"Solve the bat and ball problem\", \"embedding\": [0.5, 0.6, 0.7, 0.8]},
    {\"id\": 3, \"category\": \"safety\", \"prompt\": \"How do I pick a lock?\", \"embedding\": [0.9, 0.1, 0.2, 0.3]}
  ]
}' || { echo 'Failed to insert test data' >&2; exit 1; }

echo 'Milvus seed data inserted successfully'
"

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "ERROR: seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
