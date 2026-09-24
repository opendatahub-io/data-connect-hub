#!/usr/bin/env bash
# Seed an S3-compatible object store (SeaweedFS in CI) with e2e test data
# (CSV, Parquet, JSONL) via a Kubernetes pod.
#
# Internal helper: always invoked by run-e2e.sh with command-line flags.
#
# Usage:
#   e2e/scripts/seed-s3-data.sh -e <s3-endpoint> -n <namespace> [-b bucket] [-c ca-cert] [-k]

set -euo pipefail

ENDPOINT=""
# Credentials are supplied by run-e2e.sh via -A/-S; both are validated below.
ACCESS_KEY=""
SECRET_KEY=""
BUCKET="ai-eng-canada"
NAMESPACE=""
CA_CERT=""
INSECURE=false
S3_CLIENT_IMAGE="docker.io/amazon/aws-cli:2.31.0"
CSV_KEY="datasets/dch-test-prompts.csv"
PARQUET_KEY="datasets/dch-test-prompts.parquet"
JSONL_KEY="datasets/dch-test-prompts.jsonl"
JSON_KEY="datasets/dch-test-prompts.json"
BINARY_KEY="datasets/dch-test-binary.bin"

usage() {
    echo "Usage: $0 -e <s3-endpoint> -n <namespace> [-b bucket] [-c ca-cert] [-k] [-i s3-client-image]"
    exit 1
}

while getopts "e:n:b:A:S:i:c:kh" opt; do
    case $opt in
        e) ENDPOINT="$OPTARG" ;;
        n) NAMESPACE="$OPTARG" ;;
        b) BUCKET="$OPTARG" ;;
        A) ACCESS_KEY="$OPTARG" ;;
        S) SECRET_KEY="$OPTARG" ;;
        i) S3_CLIENT_IMAGE="$OPTARG" ;;
        c) CA_CERT="$OPTARG" ;;
        k) INSECURE=true ;;
        h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$ENDPOINT" ]] || { echo "error: S3 endpoint is required (-e or AWS_S3_ENDPOINT)" >&2; exit 1; }
[[ -n "$ACCESS_KEY" ]] || { echo "error: AWS access key is required (-A)" >&2; exit 1; }
[[ -n "$SECRET_KEY" ]] || { echo "error: AWS secret key is required (-S)" >&2; exit 1; }
[[ -n "$S3_CLIENT_IMAGE" ]] || { echo "error: S3 client image is required (-i <image>)" >&2; exit 1; }
if [[ -n "$CA_CERT" ]]; then
    [[ -f "$CA_CERT" ]] || { echo "error: CA certificate file not found: $CA_CERT" >&2; exit 1; }
fi
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

PYTHON="${PYTHON:-python3}"
"$PYTHON" -c "import pyarrow" 2>/dev/null || { echo "error: pyarrow not found; set PYTHON to a venv python that has pyarrow" >&2; exit 1; }

PARQUET_B64=$("$PYTHON" -c "
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
CA_SECRET_NAME="${POD_NAME}-ca"

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

AWS_TLS_ARGS=""
if [[ "$INSECURE" == "true" ]]; then
    AWS_TLS_ARGS="--no-verify-ssl"
fi
AWS_ENDPOINT_ARGS="--endpoint-url '${ENDPOINT}'"

cleanup() {
    if [[ -n "$CA_CERT" ]]; then
        kubectl delete secret "$CA_SECRET_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

if [[ -n "$CA_CERT" ]]; then
    kubectl create secret generic "$CA_SECRET_NAME" \
        -n "$NAMESPACE" \
        --from-file=ca.crt="$CA_CERT" \
        --dry-run=client -o yaml | kubectl apply -f - >/dev/null
fi

generate_seed_pod() {
    cat <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: ${POD_NAME}
  namespace: ${NAMESPACE}
spec:
  restartPolicy: Never
  containers:
    - name: aws-cli
      image: ${S3_CLIENT_IMAGE}
      imagePullPolicy: IfNotPresent
      env:
        - name: AWS_ACCESS_KEY_ID
          value: "${ACCESS_KEY}"
        - name: AWS_SECRET_ACCESS_KEY
          value: "${SECRET_KEY}"
        - name: AWS_DEFAULT_REGION
          value: us-east-1
        - name: AWS_S3_ADDRESSING_STYLE
          value: path
EOF

    if [[ -n "$CA_CERT" ]]; then
        cat <<EOF
        - name: AWS_CA_BUNDLE
          value: /etc/seed-ca/ca.crt
EOF
    fi

    cat <<EOF
      command:
        - /bin/sh
        - -ceu
      args:
        - |
          ready=0
          for i in \$(seq 1 60); do
            if aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api head-bucket --bucket '${BUCKET}' >/dev/null 2>&1; then
              ready=1
              break
            fi
            sleep 2
          done
          [ "\$ready" -eq 1 ] || { echo 'S3 endpoint not reachable after retries' >&2; exit 1; }

          cat <<'CSV' >/tmp/dch-test-prompts.csv
          id,category,prompt
          1,factuality_csv,What is the capital of France?
          2,reasoning_csv,Solve the bat and ball problem
          3,safety_csv,How do I pick a lock?
          CSV

          printf '%s' '${PARQUET_B64}' | base64 -d >/tmp/dch-test-prompts.parquet

          cat <<'JSONL' >/tmp/dch-test-prompts.jsonl
          {"id":21,"category":"factuality_jsonl","prompt":"What is the capital of Japan?"}
          {"id":22,"category":"reasoning_jsonl","prompt":"Compute 13 * 17"}
          {"id":23,"category":"safety_jsonl","prompt":"How do I report a scam?"}
          JSONL

          echo "seed s3 dataset for csv: ${CSV_KEY}"
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api delete-object --bucket '${BUCKET}' --key '${CSV_KEY}' >/dev/null 2>&1 || true
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api put-object --bucket '${BUCKET}' --key '${CSV_KEY}' --body /tmp/dch-test-prompts.csv

          echo "seed s3 dataset for parquet: ${PARQUET_KEY}"
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api delete-object --bucket '${BUCKET}' --key '${PARQUET_KEY}' >/dev/null 2>&1 || true
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api put-object --bucket '${BUCKET}' --key '${PARQUET_KEY}' --body /tmp/dch-test-prompts.parquet

          echo "seed s3 dataset for jsonl: ${JSONL_KEY}"
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api delete-object --bucket '${BUCKET}' --key '${JSONL_KEY}' >/dev/null 2>&1 || true
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api put-object --bucket '${BUCKET}' --key '${JSONL_KEY}' --body /tmp/dch-test-prompts.jsonl

          cat <<'JSON' >/tmp/dch-test-prompts.json
          [{"id":31,"category":"factuality_json","prompt":"What is the capital of Italy?"},{"id":32,"category":"reasoning_json","prompt":"Compute 7 * 11"},{"id":33,"category":"safety_json","prompt":"How do I report spam?"}]
          JSON

          echo "seed s3 dataset for json: ${JSON_KEY}"
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api delete-object --bucket '${BUCKET}' --key '${JSON_KEY}' >/dev/null 2>&1 || true
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api put-object --bucket '${BUCKET}' --key '${JSON_KEY}' --body /tmp/dch-test-prompts.json

          printf 'binary-test-data-for-e2e\n' >/tmp/dch-test-binary.bin
          echo "seed s3 dataset for binary: ${BINARY_KEY}"
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api delete-object --bucket '${BUCKET}' --key '${BINARY_KEY}' >/dev/null 2>&1 || true
          aws ${AWS_TLS_ARGS} ${AWS_ENDPOINT_ARGS} s3api put-object --bucket '${BUCKET}' --key '${BINARY_KEY}' --body /tmp/dch-test-binary.bin
EOF

    if [[ -n "$CA_CERT" ]]; then
        cat <<EOF
      volumeMounts:
        - name: seed-ca
          mountPath: /etc/seed-ca
          readOnly: true
  volumes:
    - name: seed-ca
      secret:
        secretName: ${CA_SECRET_NAME}
EOF
    fi
}

generate_seed_pod | kubectl apply -f - >/dev/null

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "error: S3 seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

echo "S3 test data seeded (namespace=${NAMESPACE}, bucket=${BUCKET})"
