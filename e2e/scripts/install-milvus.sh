#!/usr/bin/env bash
# Install Milvus standalone via Helm and wait for it to become ready.
#
# Usage:
#   hack/setup-milvus.sh                          # defaults: namespace=milvus, release=milvus
#   hack/setup-milvus.sh -n dch -r my-milvus      # custom namespace and release name
#
# Environment overrides (command-line flags take precedence):
#   MILVUS_NAMESPACE        target namespace         (default: milvus)
#   MILVUS_HELM_RELEASE     Helm release name        (default: milvus)
#   MILVUS_CHART_VERSION    Helm chart version       (default: 5.0.25, Milvus 2.6.x)
#   MILVUS_WAIT_TIMEOUT     kubectl wait timeout     (default: 300s)

set -euo pipefail

NAMESPACE="${MILVUS_NAMESPACE:-milvus}"
RELEASE="${MILVUS_HELM_RELEASE:-milvus}"
CHART_VERSION="${MILVUS_CHART_VERSION:-5.0.25}"
TIMEOUT="${MILVUS_WAIT_TIMEOUT:-300s}"

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-t timeout]"
    exit 1
}

while getopts "n:r:v:t:h" opt; do
    case $opt in
        n) NAMESPACE="$OPTARG" ;;
        r) RELEASE="$OPTARG" ;;
        v) CHART_VERSION="$OPTARG" ;;
        t) TIMEOUT="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

command -v helm >/dev/null || { echo "error: helm not found" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

kubectl create ns "$NAMESPACE" 2>/dev/null || true

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
        --set standalone.resources.requests.memory=512Mi \
        --set standalone.resources.requests.cpu=200m \
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
