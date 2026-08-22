#!/usr/bin/env bash
# Install Neo4j standalone via Helm and wait for it to become ready.
#
# Usage:
#   e2e/scripts/install-neo4j.sh                              # defaults: namespace=neo4j, release=neo4j
#   e2e/scripts/install-neo4j.sh -n dch -r my-neo4j           # custom namespace and release name
#
# Environment overrides (command-line flags take precedence):
#   NEO4J_NAMESPACE         target namespace         (default: neo4j)
#   NEO4J_HELM_RELEASE      Helm release name        (default: neo4j)
#   NEO4J_CHART_VERSION     Helm chart version       (default: 5.26.1)
#   NEO4J_PASSWORD          initial password         (default: testpassword)
#   NEO4J_WAIT_TIMEOUT      kubectl wait timeout     (default: 300s)

set -euo pipefail

NAMESPACE="${NEO4J_NAMESPACE:-neo4j}"
RELEASE="${NEO4J_HELM_RELEASE:-neo4j}"
CHART_VERSION="${NEO4J_CHART_VERSION:-5.26.1}"
PASSWORD="${NEO4J_PASSWORD:-testpassword}"
TIMEOUT="${NEO4J_WAIT_TIMEOUT:-300s}"

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

# Detect OpenShift (has SecurityContextConstraints API) vs vanilla Kubernetes
SECURITY_OPTS=""
if kubectl api-resources --api-group=route.openshift.io 2>/dev/null | grep -q routes; then
    echo "Detected OpenShift — clearing hardcoded UIDs for SCC compatibility"
    SECURITY_OPTS="\
        --set securityContext.runAsUser=null \
        --set securityContext.runAsGroup=null \
        --set securityContext.fsGroup=null \
        --set containerSecurityContext.runAsUser=null \
        --set containerSecurityContext.runAsGroup=null"
fi

helm repo add neo4j https://helm.neo4j.com/neo4j >/dev/null 2>&1 || true
helm repo update neo4j >/dev/null 2>&1

if helm status "$RELEASE" -n "$NAMESPACE" >/dev/null 2>&1; then
    echo "Neo4j Helm release '${RELEASE}' already exists in namespace '${NAMESPACE}'"
else
    echo "Installing Neo4j standalone via Helm (namespace=${NAMESPACE}, release=${RELEASE}, chart=${CHART_VERSION})"
    helm install "$RELEASE" neo4j/neo4j -n "$NAMESPACE" \
        --version "$CHART_VERSION" \
        --set neo4j.name="$RELEASE" \
        --set neo4j.password="$PASSWORD" \
        --set neo4j.edition=community \
        --set volumes.data.mode=defaultStorageClass \
        --set neo4j.resources.requests.memory=2Gi \
        --set neo4j.resources.requests.cpu=500m \
        $SECURITY_OPTS \
        --wait --timeout="$TIMEOUT" || {
        echo "error: failed to install Neo4j in namespace '${NAMESPACE}'" >&2
        exit 1
    }
fi

kubectl rollout status --watch --timeout="$TIMEOUT" "statefulset/${RELEASE}" -n "$NAMESPACE" || {
    kubectl get pods -n "$NAMESPACE" -l "app=${RELEASE}" || true
    echo "error: Neo4j statefulset did not become Ready in namespace '${NAMESPACE}'" >&2
    exit 1
}

echo "Neo4j is ready (namespace=${NAMESPACE}, release=${RELEASE})"
echo "  bolt://  ${RELEASE}.${NAMESPACE}.svc.cluster.local:7687"
echo "  http://  ${RELEASE}.${NAMESPACE}.svc.cluster.local:7474"
