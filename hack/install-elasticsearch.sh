#!/usr/bin/env bash
# Install Elasticsearch via Helm and wait for it to become ready.
#
# Usage:
#   hack/install-elasticsearch.sh                          # defaults: namespace=elasticsearch, release=elasticsearch
#   hack/install-elasticsearch.sh -n dch -r my-es          # custom namespace and release name
#
# Options:
#   -n NAMESPACE     target namespace         (default: elasticsearch)
#   -r RELEASE       Helm release name        (default: elasticsearch)
#   -v VERSION       Helm chart version       (default: 8.5.1, Elasticsearch 8.x)
#   -p PASSWORD     elastic user password    (required)
#   -t TIMEOUT      kubectl wait timeout     (default: 300s)
#   -h, --help      show this help
#
set -euo pipefail

NAMESPACE="elasticsearch"
RELEASE="elasticsearch"
CHART_VERSION="8.5.1"
# NOTE: no default password. The operator MUST supply -p, otherwise the
# script exits. A well-known default (e.g. "testpassword") would be a
# hard-coded credential (CWE-798) an attacker could use to log in.
PASSWORD=""
TIMEOUT="300s"

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: $1 requires an argument" >&2
        exit 1
    fi
}

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-p password] [-t timeout]"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)            require_arg "$@"; NAMESPACE="$2"; shift 2 ;;
        -r)            require_arg "$@"; RELEASE="$2"; shift 2 ;;
        -v)            require_arg "$@"; CHART_VERSION="$2"; shift 2 ;;
        -p)            require_arg "$@"; PASSWORD="$2"; shift 2 ;;
        -t)            require_arg "$@"; TIMEOUT="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "error: unknown option: $1" >&2; usage; exit 1 ;;
    esac
done

if [[ -z "${PASSWORD:-}" ]]; then
    echo "error: password is required; supply it with -p" >&2
    usage
fi

command -v helm >/dev/null || { echo "error: helm not found" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

kubectl create ns "$NAMESPACE" 2>/dev/null || true

# Detect OpenShift vs vanilla Kubernetes
SECURITY_OPTS=""
if kubectl api-resources --api-group=route.openshift.io 2>/dev/null | grep -q routes; then
    echo "Detected OpenShift — clearing hardcoded UIDs for SCC compatibility"
    SECURITY_OPTS="\
        --set sysctlInitContainer.enabled=false \
        --set podSecurityContext.fsGroup=null \
        --set podSecurityContext.runAsUser=null \
        --set securityContext.runAsUser=null \
        --set securityContext.runAsNonRoot=true"
fi

helm repo add elastic https://helm.elastic.co >/dev/null 2>&1 || true
helm repo update elastic >/dev/null 2>&1

if helm status "$RELEASE" -n "$NAMESPACE" >/dev/null 2>&1; then
    echo "Elasticsearch Helm release '${RELEASE}' already exists in namespace '${NAMESPACE}'"
else
    echo "Installing Elasticsearch via Helm (namespace=${NAMESPACE}, release=${RELEASE}, chart=${CHART_VERSION})"
    helm install "$RELEASE" elastic/elasticsearch -n "$NAMESPACE" \
        --version "$CHART_VERSION" \
        --set replicas=1 \
        --set minimumMasterNodes=1 \
        --set secret.password="$PASSWORD" \
        --set resources.requests.memory=512Mi \
        --set resources.requests.cpu=500m \
        --set persistence.enabled=true \
        $SECURITY_OPTS \
        --wait --timeout="$TIMEOUT" || {
        echo "error: failed to install Elasticsearch in namespace '${NAMESPACE}'" >&2
        exit 1
    }
fi

kubectl wait --for=condition=Ready \
    pod -l "app=elasticsearch-master,release=${RELEASE}" \
    -n "$NAMESPACE" --timeout="$TIMEOUT" >/dev/null || {
    kubectl get pods -n "$NAMESPACE" -l "release=${RELEASE}" || true
    echo "error: Elasticsearch pod did not become Ready in namespace '${NAMESPACE}'" >&2
    exit 1
}

echo "Elasticsearch is ready (namespace=${NAMESPACE}, release=${RELEASE})"
echo "  http://  ${RELEASE}-master.${NAMESPACE}.svc.cluster.local:9200"
