#!/usr/bin/env bash
# Install MinIO using a simple Kubernetes Deployment + Service.
# Optionally creates an initial bucket via a one-off mc pod.
#
# The data is intentionally ephemeral (emptyDir), making this suitable
# for E2E/integration tests.
#
# Usage:
#   hack/install-minio.sh -n dch-tenant -p secretpass
#   hack/install-minio.sh -n dch-tenant -p secretpass -b my-bucket -m minio/mc:latest
#
# Options:
#   -n NAMESPACE     target namespace           (default: minio)
#   -r RELEASE       release / resource name    (default: minio)
#   -u USER          root user name             (default: minioadmin)
#   -p PASSWORD      root password              (required)
#   -b BUCKET        bucket to create           (optional)
#   -i IMAGE         MinIO server image         (default: quay.io/minio/minio:latest)
#   -m MC_IMAGE      MinIO client image         (required if -b is specified)
#   -t TIMEOUT       rollout timeout            (default: 300s)
#   -h, --help       show this help
#
set -euo pipefail

NAMESPACE="minio"
RELEASE="minio"
USERNAME="minioadmin"
# NOTE: no default password — the caller MUST supply -p.
PASSWORD=""
BUCKET=""
IMAGE="quay.io/minio/minio:latest"
MC_IMAGE=""
TIMEOUT="300s"

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: $1 requires an argument" >&2
        exit 1
    fi
}

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-u user] [-p password] [-b bucket] [-i image] [-m mc-image] [-t timeout]"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)            require_arg "$@"; NAMESPACE="$2"; shift 2 ;;
        -r)            require_arg "$@"; RELEASE="$2"; shift 2 ;;
        -u)            require_arg "$@"; USERNAME="$2"; shift 2 ;;
        -p)            require_arg "$@"; PASSWORD="$2"; shift 2 ;;
        -b)            require_arg "$@"; BUCKET="$2"; shift 2 ;;
        -i)            require_arg "$@"; IMAGE="$2"; shift 2 ;;
        -m)            require_arg "$@"; MC_IMAGE="$2"; shift 2 ;;
        -t)            require_arg "$@"; TIMEOUT="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "error: unknown option: $1" >&2; usage; exit 1 ;;
    esac
done

if [[ -z "${PASSWORD:-}" ]]; then
    echo "error: password is required; supply it with -p" >&2
    usage
    exit 1
fi

if [[ -n "$BUCKET" && -z "$MC_IMAGE" ]]; then
    echo "error: -m MC_IMAGE is required when -b BUCKET is specified" >&2
    usage
    exit 1
fi

command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Namespace
# ---------------------------------------------------------------------------

kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# ---------------------------------------------------------------------------
# Remove old resources
# ---------------------------------------------------------------------------

kubectl delete deployment "$RELEASE" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# Deploy MinIO
# ---------------------------------------------------------------------------

echo "Installing MinIO (namespace=${NAMESPACE}, release=${RELEASE})"

kubectl apply -n "$NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Service
metadata:
  name: ${RELEASE}
  labels:
    app.kubernetes.io/name: minio
    app.kubernetes.io/instance: ${RELEASE}
spec:
  ports:
    - name: api
      port: 9000
      targetPort: 9000
    - name: console
      port: 9001
      targetPort: 9001
  selector:
    app.kubernetes.io/name: minio
    app.kubernetes.io/instance: ${RELEASE}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${RELEASE}
  labels:
    app.kubernetes.io/name: minio
    app.kubernetes.io/instance: ${RELEASE}
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: minio
      app.kubernetes.io/instance: ${RELEASE}
  template:
    metadata:
      labels:
        app.kubernetes.io/name: minio
        app.kubernetes.io/instance: ${RELEASE}
    spec:
      containers:
        - name: minio
          image: ${IMAGE}
          imagePullPolicy: IfNotPresent
          command:
            - minio
            - server
            - /data
            - --console-address
            - ":9001"
          env:
            - name: MINIO_ROOT_USER
              value: "${USERNAME}"
            - name: MINIO_ROOT_PASSWORD
              value: "${PASSWORD}"
          ports:
            - containerPort: 9000
              name: api
            - containerPort: 9001
              name: console
          resources:
            requests:
              memory: "256Mi"
              cpu: "250m"
          readinessProbe:
            httpGet:
              path: /minio/health/ready
              port: 9000
            initialDelaySeconds: 5
            periodSeconds: 5
            timeoutSeconds: 3
            failureThreshold: 12
          livenessProbe:
            httpGet:
              path: /minio/health/live
              port: 9000
            initialDelaySeconds: 10
            periodSeconds: 10
            timeoutSeconds: 3
            failureThreshold: 6
          volumeMounts:
            - name: data
              mountPath: /data
      volumes:
        - name: data
          emptyDir: {}
EOF

# ---------------------------------------------------------------------------
# Wait for rollout
# ---------------------------------------------------------------------------

if ! kubectl rollout status deployment/"$RELEASE" -n "$NAMESPACE" --timeout="$TIMEOUT"; then
    echo ""
    echo "MinIO failed to become Ready."
    kubectl get pods -n "$NAMESPACE" -l "app.kubernetes.io/instance=${RELEASE}" -o wide || true
    echo ""
    kubectl logs -n "$NAMESPACE" -l "app.kubernetes.io/instance=${RELEASE}" --tail=50 || true
    exit 1
fi

# ---------------------------------------------------------------------------
# Create bucket (optional)
# ---------------------------------------------------------------------------

if [[ -n "$BUCKET" ]]; then
    echo "Creating bucket '${BUCKET}' via mc"

    INIT_POD="minio-init-bucket"
    kubectl delete pod "$INIT_POD" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

    kubectl run "$INIT_POD" -n "$NAMESPACE" \
        --image="$MC_IMAGE" \
        --image-pull-policy=IfNotPresent \
        --restart=Never \
        --command -- sh -c "
            mc alias set myminio http://${RELEASE}:9000 '${USERNAME}' '${PASSWORD}'
            mc mb myminio/${BUCKET} --ignore-existing
        "

    kubectl wait --for=jsonpath='{.status.phase}'=Succeeded \
        "pod/$INIT_POD" -n "$NAMESPACE" --timeout=120s || {
        kubectl logs "$INIT_POD" -n "$NAMESPACE" --tail=20 || true
        echo "error: bucket creation failed" >&2
        exit 1
    }
    kubectl delete pod "$INIT_POD" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

    echo "Bucket '${BUCKET}' created"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "MinIO is ready"
echo "  namespace: ${NAMESPACE}"
echo "  release:   ${RELEASE}"
echo "  endpoint:  http://${RELEASE}.${NAMESPACE}.svc:9000"
echo "  user:      ${USERNAME}"
[[ -n "$BUCKET" ]] && echo "  bucket:    ${BUCKET}"
