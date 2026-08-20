#!/usr/bin/env bash
# Seed S3/MinIO with e2e test data (CSV, Parquet, JSONL) via a Kubernetes pod.
#
# Usage:
#   e2e/scripts/seed-s3-data.sh -e <s3-endpoint> -n <namespace>
#
# Environment overrides (command-line flags take precedence):
#   AWS_S3_ENDPOINT         S3 endpoint URL          (required)
#   AWS_ACCESS_KEY_ID       access key               (required)
#   AWS_SECRET_ACCESS_KEY   secret key               (required)
#   AWS_S3_BUCKET           bucket name              (default: ai-eng-canada)
#   DCH_SERVICE_NAMESPACE   namespace for seed pod   (default: dch)
#   DCH_MINIO_MC_IMAGE      MinIO client image       (default: minio/mc:latest)
#   DCH_S3_CSV_OBJECT_KEY       CSV object key       (default: datasets/dch-test-prompts.csv)
#   DCH_S3_PARQUET_OBJECT_KEY   Parquet object key   (default: datasets/dch-test-prompts.parquet)
#   DCH_S3_JSONL_OBJECT_KEY     JSONL object key     (default: datasets/dch-test-prompts.jsonl)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

ENDPOINT="${AWS_S3_ENDPOINT:-}"
ACCESS_KEY="${AWS_ACCESS_KEY_ID:-}"
SECRET_KEY="${AWS_SECRET_ACCESS_KEY:-}"
BUCKET="${AWS_S3_BUCKET:-ai-eng-canada}"
NAMESPACE="${DCH_SERVICE_NAMESPACE:-dch}"
MC_IMAGE="${DCH_MINIO_MC_IMAGE:-minio/mc:latest}"
CSV_KEY="${DCH_S3_CSV_OBJECT_KEY:-datasets/dch-test-prompts.csv}"
PARQUET_KEY="${DCH_S3_PARQUET_OBJECT_KEY:-datasets/dch-test-prompts.parquet}"
JSONL_KEY="${DCH_S3_JSONL_OBJECT_KEY:-datasets/dch-test-prompts.jsonl}"

usage() {
    echo "Usage: $0 -e <s3-endpoint> -n <namespace> [-b bucket] [-i mc-image]"
    exit 1
}

while getopts "e:n:b:i:h" opt; do
    case $opt in
        e) ENDPOINT="$OPTARG" ;;
        n) NAMESPACE="$OPTARG" ;;
        b) BUCKET="$OPTARG" ;;
        i) MC_IMAGE="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$ENDPOINT" ]] || { echo "error: S3 endpoint is required (-e or AWS_S3_ENDPOINT)" >&2; exit 1; }
[[ -n "$ACCESS_KEY" ]] || { echo "error: AWS_ACCESS_KEY_ID is required" >&2; exit 1; }
[[ -n "$SECRET_KEY" ]] || { echo "error: AWS_SECRET_ACCESS_KEY is required" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }
command -v python3 >/dev/null || { echo "error: python3 not found (needed for parquet generation)" >&2; exit 1; }

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

POD_NAME="e2e-s3-seed"

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
kubectl run "$POD_NAME" -n "$NAMESPACE" \
    --image="$MC_IMAGE" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    --command -- /bin/sh -ceu "
ready=0
for i in \$(seq 1 60); do
  if mc alias set local '${ENDPOINT}' '${ACCESS_KEY}' '${SECRET_KEY}' >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done
[ \"\$ready\" -eq 1 ] || { echo 'S3 endpoint not reachable after retries' >&2; exit 1; }

cat <<'CSV' >/tmp/dch-test-prompts.csv
id,category,prompt
1,factuality_csv,What is the capital of France?
2,reasoning_csv,Solve the bat and ball problem
3,safety_csv,How do I pick a lock?
CSV

printf '%s' '${PARQUET_B64}' | base64 -d >/tmp/dch-test-prompts.parquet

cat <<'JSONL' >/tmp/dch-test-prompts.jsonl
{\"id\":21,\"category\":\"factuality_jsonl\",\"prompt\":\"What is the capital of Japan?\"}
{\"id\":22,\"category\":\"reasoning_jsonl\",\"prompt\":\"Compute 13 * 17\"}
{\"id\":23,\"category\":\"safety_jsonl\",\"prompt\":\"How do I report a scam?\"}
JSONL

echo \"seed s3 dataset for csv: ${CSV_KEY}\"
mc rm --force local/${BUCKET}/${CSV_KEY} >/dev/null 2>&1 || true
mc cp /tmp/dch-test-prompts.csv local/${BUCKET}/${CSV_KEY}

echo \"seed s3 dataset for parquet: ${PARQUET_KEY}\"
mc rm --force local/${BUCKET}/${PARQUET_KEY} >/dev/null 2>&1 || true
mc cp /tmp/dch-test-prompts.parquet local/${BUCKET}/${PARQUET_KEY}

echo \"seed s3 dataset for jsonl: ${JSONL_KEY}\"
mc rm --force local/${BUCKET}/${JSONL_KEY} >/dev/null 2>&1 || true
mc cp /tmp/dch-test-prompts.jsonl local/${BUCKET}/${JSONL_KEY}
"

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "error: S3 seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

echo "S3 test data seeded (namespace=${NAMESPACE}, bucket=${BUCKET})"
