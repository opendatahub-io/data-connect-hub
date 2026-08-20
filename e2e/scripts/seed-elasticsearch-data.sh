#!/usr/bin/env bash
# Seed Elasticsearch with e2e test data via a Kubernetes pod.
#
# Usage:
#   e2e/scripts/seed-elasticsearch-data.sh -e https://elasticsearch-master:9200 -n <namespace> -p <password>
#
# Environment overrides (command-line flags take precedence):
#   DCH_ES_URI              Elasticsearch endpoint   (required)
#   DCH_ES_PASSWORD         elastic user password    (optional)
#   DCH_SERVICE_NAMESPACE   Namespace for seed pod   (default: dch)

set -euo pipefail

ENDPOINT="${DCH_ES_URI:-}"
PASSWORD="${DCH_ES_PASSWORD:-}"
NAMESPACE="${DCH_SERVICE_NAMESPACE:-dch}"
INDEX="dch_e2e_cities"

usage() {
    echo "Usage: $0 -e <elasticsearch-uri> [-n namespace] [-p password]"
    exit 1
}

while getopts "e:n:p:h" opt; do
    case $opt in
        e) ENDPOINT="$OPTARG" ;;
        n) NAMESPACE="$OPTARG" ;;
        p) PASSWORD="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$ENDPOINT" ]] || { echo "error: Elasticsearch URI is required (-e or DCH_ES_URI)" >&2; exit 1; }

POD_NAME="e2e-es-seed"

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
kubectl run "$POD_NAME" -n "$NAMESPACE" \
    --image="curlimages/curl:latest" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    --command -- /bin/sh -ceu "
ES_URI='${ENDPOINT}'
PASSWORD='${PASSWORD}'
INDEX='${INDEX}'

AUTH=''
[ -n \"\${PASSWORD}\" ] && AUTH=\"-u elastic:\${PASSWORD}\"

# Wait for Elasticsearch to be ready (-k to skip cert verification)
ready=0
for i in \$(seq 1 60); do
  if curl -ksf \${AUTH} \"\${ES_URI}/_cluster/health\" >/dev/null 2>&1; then
    ready=1; break
  fi
  echo \"Waiting for Elasticsearch... (attempt \${i}/60)\"
  sleep 2
done
[ \"\$ready\" -eq 1 ] || { echo 'Elasticsearch not reachable' >&2; exit 1; }

# Delete existing test index
curl -ksf \${AUTH} -X DELETE \"\${ES_URI}/\${INDEX}\" >/dev/null 2>&1 || true

# Create index with mapping
curl -ksf \${AUTH} -X PUT \"\${ES_URI}/\${INDEX}\" \
  -H 'Content-Type: application/json' \
  -d '{
  \"settings\": {
    \"number_of_shards\": 1,
    \"number_of_replicas\": 0
  },
  \"mappings\": {
    \"properties\": {
      \"name\":       {\"type\": \"keyword\"},
      \"country\":    {\"type\": \"keyword\"},
      \"population\": {\"type\": \"integer\"},
      \"location\":   {\"type\": \"geo_point\"},
      \"description\":{\"type\": \"text\"}
    }
  }
}' || { echo 'Failed to create index' >&2; exit 1; }

# Bulk insert test data
curl -ksf \${AUTH} -X POST \"\${ES_URI}/\${INDEX}/_bulk\" \
  -H 'Content-Type: application/x-ndjson' \
  -d '
{\"index\":{\"_id\":\"1\"}}
{\"name\":\"Tokyo\",\"country\":\"Japan\",\"population\":13960000,\"location\":{\"lat\":35.6762,\"lon\":139.6503},\"description\":\"Capital of Japan and the most populous metropolitan area in the world\"}
{\"index\":{\"_id\":\"2\"}}
{\"name\":\"London\",\"country\":\"United Kingdom\",\"population\":8982000,\"location\":{\"lat\":51.5074,\"lon\":-0.1278},\"description\":\"Capital of England and the United Kingdom\"}
{\"index\":{\"_id\":\"3\"}}
{\"name\":\"Paris\",\"country\":\"France\",\"population\":2161000,\"location\":{\"lat\":48.8566,\"lon\":2.3522},\"description\":\"Capital of France known for art culture and the Eiffel Tower\"}
{\"index\":{\"_id\":\"4\"}}
{\"name\":\"New York\",\"country\":\"United States\",\"population\":8336000,\"location\":{\"lat\":40.7128,\"lon\":-74.0060},\"description\":\"The most populous city in the United States\"}
{\"index\":{\"_id\":\"5\"}}
{\"name\":\"Berlin\",\"country\":\"Germany\",\"population\":3645000,\"location\":{\"lat\":52.5200,\"lon\":13.4050},\"description\":\"Capital and largest city of Germany\"}
' || { echo 'Failed to insert test data' >&2; exit 1; }

# Refresh index to make data searchable immediately
curl -ksf \${AUTH} -X POST \"\${ES_URI}/\${INDEX}/_refresh\" >/dev/null

echo 'Elasticsearch seed data inserted successfully'
"

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "ERROR: seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
