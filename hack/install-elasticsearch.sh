#!/usr/bin/env bash
# Install Elasticsearch via Helm and wait for it to become ready.
#
# Usage:
#   test/install-elasticsearch.sh                          # defaults: namespace=elasticsearch, release=elasticsearch
#   test/install-elasticsearch.sh -n dch -r my-es          # custom namespace and release name
#
# Environment overrides (command-line flags take precedence):
#   ES_NAMESPACE            target namespace         (default: elasticsearch)
#   ES_HELM_RELEASE         Helm release name        (default: elasticsearch)
#   ES_CHART_VERSION        Helm chart version       (default: 8.5.1, Elasticsearch 8.x)
#   ES_PASSWORD             elastic user password    (default: testpassword)
#   ES_WAIT_TIMEOUT         kubectl wait timeout     (default: 300s)

set -euo pipefail

NAMESPACE="${ES_NAMESPACE:-elasticsearch}"
RELEASE="${ES_HELM_RELEASE:-elasticsearch}"
CHART_VERSION="${ES_CHART_VERSION:-8.5.1}"
PASSWORD="${ES_PASSWORD:-testpassword}"
TIMEOUT="${ES_WAIT_TIMEOUT:-300s}"

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-p password] [-t timeout]"
    exit 1
}

while getopts "n:r:v:p:t:h" opt; do
    case $opt in
        n) NAMESPACE="$OPTARG" ;;
        r) RELEASE="$OPTARG" ;;
        v) CHART_VERSION="$OPTARG" ;;
        p) PASSWORD="$OPTARG" ;;
        t) TIMEOUT="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
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
        --set resources.requests.memory=1Gi \
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
