#!/usr/bin/env bash
# Seed an OCI registry with e2e test data (CSV, Parquet, JSONL) via oras.
#
# Usage:
#   e2e/scripts/seed-oci-data.sh -r <registry-host> -n <namespace>
#
# Environment overrides (command-line flags take precedence):
#   DCH_OCI_HOST              OCI registry host         (required)
#   DCH_OCI_USERNAME          registry username          (optional, for auth)
#   DCH_OCI_PASSWORD          registry password          (optional, for auth)
#   DCH_SERVICE_NAMESPACE     namespace for seed pod     (default: dch)
#   DCH_OCI_ORAS_IMAGE        oras CLI image             (default: ghcr.io/oras-project/oras:v1.2.2)
#   DCH_OCI_CSV_REPO          CSV artifact repo          (default: dch-test/prompts-csv)
#   DCH_OCI_PARQUET_REPO      Parquet artifact repo      (default: dch-test/prompts-parquet)
#   DCH_OCI_JSONL_REPO        JSONL artifact repo        (default: dch-test/prompts-jsonl)
#   DCH_OCI_BINARY_REPO       Binary artifact repo       (default: dch-test/prompts-binary)

set -euo pipefail

REGISTRY="${DCH_OCI_HOST:-}"
USERNAME="${DCH_OCI_USERNAME:-}"
PASSWORD="${DCH_OCI_PASSWORD:-}"
NAMESPACE="${DCH_SERVICE_NAMESPACE:-dch}"
ORAS_IMAGE="${DCH_OCI_ORAS_IMAGE:-ghcr.io/oras-project/oras:v1.2.2}"
CSV_REPO="${DCH_OCI_CSV_REPO:-dch-test/prompts-csv}"
PARQUET_REPO="${DCH_OCI_PARQUET_REPO:-dch-test/prompts-parquet}"
JSONL_REPO="${DCH_OCI_JSONL_REPO:-dch-test/prompts-jsonl}"
BINARY_REPO="${DCH_OCI_BINARY_REPO:-dch-test/prompts-binary}"
TAG="v1"

usage() {
    echo "Usage: $0 -r <registry-host> -n <namespace>"
    exit 1
}

while getopts "r:n:h" opt; do
    case $opt in
        r) REGISTRY="$OPTARG" ;;
        n) NAMESPACE="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$REGISTRY" ]] || { echo "error: OCI registry host is required (-r or DCH_OCI_HOST)" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }
command -v python3 >/dev/null || { echo "error: python3 not found (needed for parquet generation)" >&2; exit 1; }

# Generate parquet data as base64
PARQUET_B64=$(python3 -c "
import base64, io
import pyarrow as pa, pyarrow.parquet as pq
table = pa.table({
    'id': [11, 12, 13],
    'category': ['factuality_parquet', 'reasoning_parquet', 'safety_parquet'],
    'prompt': ['What is the capital of Germany?', 'Compute 17 * 19', 'How do I report a phishing email?'],
})
buf = io.BytesIO()
pq.write_table(table, buf)
print(base64.b64encode(buf.getvalue()).decode('ascii'))
") || { echo "error: failed to generate parquet data (python3 + pyarrow required)" >&2; exit 1; }

# Build login args
LOGIN_ARGS=""
if [[ -n "$USERNAME" && -n "$PASSWORD" ]]; then
    LOGIN_ARGS="oras login '${REGISTRY}' -u '${USERNAME}' -p '${PASSWORD}' --insecure &&"
fi

# --insecure skips TLS verification (acceptable for ephemeral seed pod).
# --plain-http forces HTTP when the registry has no TLS at all.
ORAS_FLAGS=""
if [[ "${DCH_OCI_PLAIN_HTTP:-}" == "true" ]]; then
    ORAS_FLAGS="--plain-http"
else
    ORAS_FLAGS="--insecure"
fi

POD_NAME="e2e-oci-seed"

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
kubectl run "$POD_NAME" -n "$NAMESPACE" \
    --image="$ORAS_IMAGE" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    --command -- /bin/sh -ceu "
${LOGIN_ARGS}

cat <<'CSV' >/tmp/prompts.csv
id,category,prompt
1,factuality_csv,What is the capital of France?
2,reasoning_csv,Solve the bat and ball problem
3,safety_csv,How do I pick a lock?
CSV

printf '%s' '${PARQUET_B64}' | base64 -d >/tmp/prompts.parquet

cat <<'JSONL' >/tmp/prompts.jsonl
{\"id\":21,\"category\":\"factuality_jsonl\",\"prompt\":\"What is the capital of Japan?\"}
{\"id\":22,\"category\":\"reasoning_jsonl\",\"prompt\":\"Compute 13 * 17\"}
{\"id\":23,\"category\":\"safety_jsonl\",\"prompt\":\"How do I report a scam?\"}
JSONL

echo 'hello world binary test data' >/tmp/prompts.bin

cd /tmp

echo \"Pushing CSV artifact...\"
oras push ${ORAS_FLAGS} ${REGISTRY}/${CSV_REPO}:${TAG} \
  --artifact-type application/vnd.dch.dataset \
  prompts.csv:text/csv

echo \"Pushing Parquet artifact...\"
oras push ${ORAS_FLAGS} ${REGISTRY}/${PARQUET_REPO}:${TAG} \
  --artifact-type application/vnd.dch.dataset \
  prompts.parquet:application/vnd.apache.parquet

echo \"Pushing JSONL artifact...\"
oras push ${ORAS_FLAGS} ${REGISTRY}/${JSONL_REPO}:${TAG} \
  --artifact-type application/vnd.dch.dataset \
  prompts.jsonl:application/x-ndjson

echo \"Pushing binary artifact...\"
oras push ${ORAS_FLAGS} ${REGISTRY}/${BINARY_REPO}:${TAG} \
  --artifact-type application/vnd.dch.dataset \
  prompts.bin:application/octet-stream

echo 'All OCI artifacts pushed successfully'
"

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "error: OCI seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

echo "OCI test data seeded (namespace=${NAMESPACE}, registry=${REGISTRY})"
