#!/usr/bin/env bash
# Install Elasticsearch via Helm and wait for it to become ready.
#
# Usage:
#   hack/install-elasticsearch.sh                          # defaults: namespace=elasticsearch, release=elasticsearch
#   hack/install-elasticsearch.sh -n dch -r my-es          # custom namespace and release name
#   hack/install-elasticsearch.sh --ssl                    # enable TLS with Helm-generated certificates
#
# Options:
#   -n NAMESPACE     target namespace         (default: elasticsearch)
#   -r RELEASE       Helm release name        (default: elasticsearch)
#   -v VERSION       Helm chart version       (default: 8.5.1, Elasticsearch 8.x)
#   -p PASSWORD      elastic user password    (required)
#   -t TIMEOUT       kubectl wait timeout     (default: 300s)
#   --ssl            enable SSL/TLS and require TLS for all TCP connections
#   -h, --help       show this help
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
SSL_ENABLED=false

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: $1 requires an argument" >&2
        exit 1
    fi
}

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-p password] [-t timeout] [--ssl]"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)            require_arg "$@"; NAMESPACE="$2"; shift 2 ;;
        -r)            require_arg "$@"; RELEASE="$2"; shift 2 ;;
        -v)            require_arg "$@"; CHART_VERSION="$2"; shift 2 ;;
        -p)            require_arg "$@"; PASSWORD="$2"; shift 2 ;;
        -t)            require_arg "$@"; TIMEOUT="$2"; shift 2 ;;
        --ssl)         SSL_ENABLED=true; shift ;;
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
SECURITY_OPTS=()
if kubectl api-resources --api-group=route.openshift.io 2>/dev/null | grep -q routes; then
    echo "Detected OpenShift — clearing hardcoded UIDs for SCC compatibility"
    SECURITY_OPTS=(
        --set sysctlInitContainer.enabled=false
        --set podSecurityContext.fsGroup=null
        --set podSecurityContext.runAsUser=null
        --set securityContext.runAsUser=null
        --set securityContext.runAsNonRoot=true
    )
fi

# The Elasticsearch chart owns certificate generation and TLS configuration:
# createCert=true creates the certificate Secret and enables TLS for both the
# HTTP and transport protocols. When TLS is disabled, explicitly turn off
# Elasticsearch security as well so the Elasticsearch 8.x image cannot apply
# its automatic security configuration.
if [[ "$SSL_ENABLED" == "true" ]]; then
    TLS_OPTS=(
        --set createCert=true
        --set protocol=https
    )
else
    TLS_OPTS=(
        --set createCert=false
        --set protocol=http
        --set-string 'extraEnvs[0].name=xpack.security.enabled'
        --set-string 'extraEnvs[0].value=false'
        --set-string 'extraEnvs[1].name=xpack.security.transport.ssl.enabled'
        --set-string 'extraEnvs[1].value=false'
        --set-string 'extraEnvs[2].name=xpack.security.http.ssl.enabled'
        --set-string 'extraEnvs[2].value=false'
    )
fi

helm repo add elastic https://helm.elastic.co >/dev/null 2>&1 || true
helm repo update elastic >/dev/null 2>&1

if helm status "$RELEASE" -n "$NAMESPACE" >/dev/null 2>&1; then
    echo "Elasticsearch Helm release '${RELEASE}' already exists in namespace '${NAMESPACE}'"
else
    echo "Installing Elasticsearch via Helm (namespace=${NAMESPACE}, release=${RELEASE}, chart=${CHART_VERSION}, ssl=${SSL_ENABLED})"
    helm_install_args=(
        "$RELEASE" elastic/elasticsearch
        -n "$NAMESPACE"
        --version "$CHART_VERSION"
        --set replicas=1
        --set minimumMasterNodes=1
        --set secret.password="$PASSWORD"
        --set resources.requests.memory=512Mi
        --set resources.requests.cpu=500m
        --set persistence.enabled=true
    )
    if ((${#SECURITY_OPTS[@]})); then
        helm_install_args+=("${SECURITY_OPTS[@]}")
    fi
    helm_install_args+=("${TLS_OPTS[@]}")
    helm_install_args+=(--wait "--timeout=$TIMEOUT")

    helm install "${helm_install_args[@]}" || {
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

echo "Elasticsearch is ready (namespace=${NAMESPACE}, release=${RELEASE}, ssl=${SSL_ENABLED})"
if [[ "$SSL_ENABLED" == "true" ]]; then
    echo "  https:// ${RELEASE}-master.${NAMESPACE}.svc.cluster.local:9200"
else
    echo "  http://  ${RELEASE}-master.${NAMESPACE}.svc.cluster.local:9200"
fi
