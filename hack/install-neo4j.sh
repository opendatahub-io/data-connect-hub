#!/usr/bin/env bash
# Install Neo4j standalone via Helm and wait for it to become ready.
#
# Usage:
#   hack/install-neo4j.sh                              # defaults: namespace=neo4j, release=neo4j
#   hack/install-neo4j.sh -n dch -r my-neo4j           # custom namespace and release name
#
# Options:
#   -n NAMESPACE     target namespace          (default: neo4j)
#   -r RELEASE       Helm release name         (default: neo4j)
#   -v VERSION       Helm chart version        (default: 5.26.1)
#   -p PASSWORD     initial password          (required)
#   -t TIMEOUT      kubectl wait timeout       (default: 300s)
#   -s, --ssl       enable SSL/TLS and require TLS
#   --ssl-cert FILE server certificate (PEM)
#   --ssl-key FILE  server private key (PEM)
#   --ssl-ca FILE   CA certificate (PEM)
#   -h, --help      show this help
#
set -euo pipefail

NAMESPACE="neo4j"
RELEASE="neo4j"
CHART_VERSION="5.26.1"
# NOTE: no default password. The operator MUST supply -p, otherwise the
# script exits. A well-known default (e.g. "testpassword") would be a
# hard-coded credential (CWE-798) an attacker could use to log in.
PASSWORD=""
TIMEOUT="300s"

SSL_ENABLED=false
SSL_CERT=""
SSL_KEY=""
SSL_CA=""

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: $1 requires an argument" >&2
        exit 1
    fi
}

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-p password] \
[-t timeout] [--ssl|--ssl-cert FILE|--ssl-key FILE|--ssl-ca FILE]"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)            require_arg "$@"; NAMESPACE="$2"; shift 2 ;;
        -r)            require_arg "$@"; RELEASE="$2"; shift 2 ;;
        -v)            require_arg "$@"; CHART_VERSION="$2"; shift 2 ;;
        -p)            require_arg "$@"; PASSWORD="$2"; shift 2 ;;
        -t)            require_arg "$@"; TIMEOUT="$2"; shift 2 ;;
        -s|--ssl)      SSL_ENABLED=true; shift ;;
        --ssl-cert)    require_arg "$@"; SSL_CERT="$2"; shift 2 ;;
        --ssl-key)     require_arg "$@"; SSL_KEY="$2"; shift 2 ;;
        --ssl-ca)      require_arg "$@"; SSL_CA="$2"; shift 2 ;;
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

# TLS configuration: wire the parsed SSL options into the Neo4j chart.
# The chart enables TLS only through Kubernetes Secret references under
# ssl.*; without them --ssl has no effect and TLS stays disabled.
SSL_OPTS=""
if [[ "$SSL_ENABLED" == "true" ]]; then
    if [[ -z "$SSL_CERT" || -z "$SSL_KEY" ]]; then
        echo "error: --ssl requires --ssl-cert and --ssl-key" >&2
        usage
        exit 1
    fi
    [[ -f "$SSL_CERT" ]] || { echo "error: certificate file not found: $SSL_CERT" >&2; exit 1; }
    [[ -f "$SSL_KEY" ]]  || { echo "error: private key file not found: $SSL_KEY" >&2; exit 1; }
    if [[ -n "$SSL_CA" ]]; then
        [[ -f "$SSL_CA" ]] || { echo "error: CA certificate file not found: $SSL_CA" >&2; exit 1; }
    fi

    SECRET_NAME="${RELEASE}-tls"
    # Upsert a kubernetes.io/tls secret holding the server certificate and key.
    kubectl create secret tls "$SECRET_NAME" \
        --cert="$SSL_CERT" --key="$SSL_KEY" \
        -n "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f - -n "$NAMESPACE" >/dev/null

    SSL_OPTS="\
        --set ssl.bolt.privateKey.secretName=${SECRET_NAME} \
        --set ssl.bolt.privateKey.subPath=tls.key \
        --set ssl.bolt.publicCertificate.secretName=${SECRET_NAME} \
        --set ssl.bolt.publicCertificate.subPath=tls.crt"

    if [[ -n "$SSL_CA" ]]; then
        CA_SECRET_NAME="${RELEASE}-tls-ca"
        # An Opaque secret stores the CA so it can be trusted over https/cluster.
        kubectl create secret generic "$CA_SECRET_NAME" \
            --from-file=ca.crt="$SSL_CA" \
            -n "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f - -n "$NAMESPACE" >/dev/null

        SSL_OPTS="${SSL_OPTS} \
        --set 'ssl.https.trustedCerts.sources[0].secret.name=${CA_SECRET_NAME}' \
        --set 'ssl.https.trustedCerts.sources[0].secret.items[0].key=ca.crt' \
        --set 'ssl.https.trustedCerts.sources[0].secret.items[0].path=public.crt'"
    fi
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
        $SSL_OPTS \
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
