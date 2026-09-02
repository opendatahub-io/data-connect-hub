#!/usr/bin/env bash
# Install Milvus standalone via Helm and wait for it to become ready.
#
# Usage:
#   hack/setup-milvus.sh                          # defaults: namespace=milvus, release=milvus
#   hack/setup-milvus.sh -n dch -r my-milvus      # custom namespace and release name
#
# Options:
#   -n NAMESPACE     target namespace         (default: milvus)
#   -r RELEASE       Helm release name        (default: milvus)
#   -v VERSION       Helm chart version       (default: 5.0.25, Milvus 2.6.x)
#   -t TIMEOUT      kubectl wait timeout     (default: 300s)
#   -h, --help      show this help
#
set -euo pipefail

NAMESPACE="milvus"
RELEASE="milvus"
CHART_VERSION="5.0.25"
TIMEOUT="300s"

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: $1 requires an argument" >&2
        exit 1
    fi
}

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-t timeout]"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)            require_arg "$@"; NAMESPACE="$2"; shift 2 ;;
        -r)            require_arg "$@"; RELEASE="$2"; shift 2 ;;
        -v)            require_arg "$@"; CHART_VERSION="$2"; shift 2 ;;
        -t)            require_arg "$@"; TIMEOUT="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "error: unknown option: $1" >&2; usage; exit 1 ;;
    esac
done

command -v helm >/dev/null || { echo "error: helm not found" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

kubectl create ns "$NAMESPACE" 2>/dev/null || true

# Detect OpenShift vs vanilla Kubernetes
SECURITY_OPTS=""
if kubectl api-resources --api-group=route.openshift.io 2>/dev/null | grep -q routes; then
    echo "Detected OpenShift — clearing hardcoded UIDs for SCC compatibility"
    SECURITY_OPTS="\
        --set etcd.containerSecurityContext.runAsUser=null \
        --set etcd.containerSecurityContext.runAsNonRoot=true \
        --set etcd.podSecurityContext.fsGroup=null \
        --set minio.podSecurityContext.fsGroup=null \
        --set minio.containerSecurityContext.runAsUser=null \
        --set minio.containerSecurityContext.runAsNonRoot=true"
fi

helm repo add milvus https://zilliztech.github.io/milvus-helm/ >/dev/null 2>&1 || true
helm repo update milvus >/dev/null 2>&1

if helm status "$RELEASE" -n "$NAMESPACE" >/dev/null 2>&1; then
    echo "Milvus Helm release '${RELEASE}' already exists in namespace '${NAMESPACE}'"
else
    echo "Installing Milvus standalone via Helm (namespace=${NAMESPACE}, release=${RELEASE}, chart=${CHART_VERSION})"
    helm install "$RELEASE" milvus/milvus -n "$NAMESPACE" \
        --version "$CHART_VERSION" \
        --set cluster.enabled=false \
        --set streaming.messageQueue=rocksmq \
        --set pulsarv3.enabled=false \
        --set etcd.replicaCount=1 \
        --set minio.mode=standalone \
        --set minio.resources.requests.memory=512Mi \
        --set standalone.resources.requests.memory=512Mi \
        --set standalone.resources.requests.cpu=200m \
        $SECURITY_OPTS \
        --wait --timeout="$TIMEOUT" || {
        echo "error: failed to install Milvus in namespace '${NAMESPACE}'" >&2
        exit 1
    }
fi

kubectl wait --for=condition=Ready \
    pod -l "app.kubernetes.io/instance=${RELEASE},component=standalone" \
    -n "$NAMESPACE" --timeout="$TIMEOUT" >/dev/null || {
    kubectl get pods -n "$NAMESPACE" -l "app.kubernetes.io/instance=${RELEASE}" || true
    echo "error: Milvus standalone pod did not become Ready in namespace '${NAMESPACE}'" >&2
    exit 1
}

echo "Milvus is ready (namespace=${NAMESPACE}, release=${RELEASE})"
