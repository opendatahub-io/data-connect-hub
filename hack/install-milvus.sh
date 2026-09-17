#!/usr/bin/env bash
# Install Milvus standalone via Helm and wait for it to become ready.
#
# Usage:
#   hack/install-milvus.sh                        # defaults: namespace=milvus, release=milvus
#   hack/install-milvus.sh -n dch -r my-milvus    # custom namespace and release name
#   hack/install-milvus.sh --ssl                  # enable TLS with self-signed certs
#
# Options:
#   -n NAMESPACE     target namespace         (default: milvus)
#   -r RELEASE       Helm release name        (default: milvus)
#   -v VERSION       Helm chart version       (default: 5.0.25, Milvus 2.6.x)
#   -t TIMEOUT      kubectl wait timeout     (default: 300s)
#   --ssl               enable SSL/TLS and require TLS
#   --ssl-cert FILE     server certificate (PEM)
#   --ssl-key FILE      server private key (PEM)
#   --ssl-ca FILE       CA certificate (PEM)
#   -h, --help          show this help
#
# SSL:
#   --ssl without --ssl-cert/--ssl-key
#       Automatically generates a self-signed CA and server certificate.
#
#   --ssl-cert + --ssl-key
#       Use a user-provided server certificate and private key.
#
#   --ssl-ca
#       Optional CA certificate for client-side server certificate verification.
#
set -euo pipefail

NAMESPACE="milvus"
RELEASE="milvus"
CHART_VERSION="5.0.25"
TIMEOUT="300s"

TLS_ENABLED="false"
TLS_CERT=""
TLS_KEY=""
TLS_CA=""

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: $1 requires an argument" >&2
        exit 1
    fi
}

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-t timeout] \
[--ssl|--ssl-cert FILE|--ssl-key FILE|--ssl-ca FILE]"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)            require_arg "$@"; NAMESPACE="$2"; shift 2 ;;
        -r)            require_arg "$@"; RELEASE="$2"; shift 2 ;;
        -v)            require_arg "$@"; CHART_VERSION="$2"; shift 2 ;;
        -t)            require_arg "$@"; TIMEOUT="$2"; shift 2 ;;
        --ssl)         TLS_ENABLED="true"; shift ;;
        --ssl-cert)    require_arg "$@"; TLS_CERT="$2"; shift 2 ;;
        --ssl-key)     require_arg "$@"; TLS_KEY="$2"; shift 2 ;;
        --ssl-ca)      require_arg "$@"; TLS_CA="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "error: unknown option: $1" >&2; usage; exit 1 ;;
    esac
done

command -v helm >/dev/null || { echo "error: helm not found" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

kubectl create ns "$NAMESPACE" 2>/dev/null || true

# Generate self-signed TLS certificates or use user-provided certificates.
TLS_OPTS=()
if [[ -n "$TLS_CERT" || -n "$TLS_KEY" || -n "$TLS_CA" ]]; then
    TLS_ENABLED="true"
fi

if [[ "$TLS_ENABLED" == "true" ]]; then
    command -v openssl >/dev/null || { echo "error: openssl not found (required for TLS)" >&2; exit 1; }

    if [[ -n "$TLS_CERT" && -z "$TLS_KEY" ]] ||
       [[ -z "$TLS_CERT" && -n "$TLS_KEY" ]]; then
        echo "error: --ssl-cert and --ssl-key must be provided together" >&2
        exit 1
    fi

    if [[ -n "$TLS_CA" && -z "$TLS_CERT" ]]; then
        echo "error: --ssl-ca requires --ssl-cert and --ssl-key" >&2
        exit 1
    fi

    [[ -z "$TLS_CERT" || -f "$TLS_CERT" ]] || {
        echo "error: certificate file not found: $TLS_CERT" >&2
        exit 1
    }
    [[ -z "$TLS_KEY" || -f "$TLS_KEY" ]] || {
        echo "error: private key file not found: $TLS_KEY" >&2
        exit 1
    }
    [[ -z "$TLS_CA" || -f "$TLS_CA" ]] || {
        echo "error: CA certificate file not found: $TLS_CA" >&2
        exit 1
    }

    CERT_DIR="$(mktemp -d)"
    trap 'rm -rf "$CERT_DIR"' EXIT

    if [[ -z "$TLS_CERT" ]]; then
        echo "Generating self-signed TLS certificates in ${CERT_DIR}"

        # CA key and certificate
        openssl genrsa -out "${CERT_DIR}/ca.key" 2048 >/dev/null 2>&1
        openssl req -x509 -new -nodes -key "${CERT_DIR}/ca.key" -sha256 -days 3650 \
            -out "${CERT_DIR}/ca.pem" \
            -subj "/C=US/ST=CA/L=SanFrancisco/O=DataConnectHub/CN=MilvusCA"

        # Server key and certificate signed by CA
        openssl genrsa -out "${CERT_DIR}/server.key" 2048 >/dev/null 2>&1

        cat > "${CERT_DIR}/openssl.cnf" <<'SSLCNF'
[req]
distinguished_name = req_dn
req_extensions = v3_req
[req_dn]
[v3_req]
subjectAltName = @alt_names
[alt_names]
DNS.1 = localhost
DNS.2 = *.milvus.svc.cluster.local
DNS.3 = *.milvus
SSLCNF
        # Replace placeholder namespace in SAN entries
        sed -i.bak "s/\.milvus/.${NAMESPACE}/g" "${CERT_DIR}/openssl.cnf"

        openssl req -new -key "${CERT_DIR}/server.key" \
            -subj "/C=US/ST=CA/L=SanFrancisco/O=DataConnectHub/CN=localhost" \
            | openssl x509 -req -days 3650 -out "${CERT_DIR}/server.pem" \
                -CA "${CERT_DIR}/ca.pem" -CAkey "${CERT_DIR}/ca.key" -CAcreateserial \
                -extfile "${CERT_DIR}/openssl.cnf" -extensions v3_req 2>/dev/null

        TLS_CERT="${CERT_DIR}/server.pem"
        TLS_KEY="${CERT_DIR}/server.key"
        TLS_CA="${CERT_DIR}/ca.pem"
    fi

    TLS_SECRET_ARGS=(
        "--from-file=server.pem=${TLS_CERT}"
        "--from-file=server.key=${TLS_KEY}"
    )
    if [[ -n "$TLS_CA" ]]; then
        TLS_SECRET_ARGS+=("--from-file=ca.pem=${TLS_CA}")
    fi

    kubectl create secret generic "${RELEASE}-milvus-tls" \
        -n "$NAMESPACE" \
        "${TLS_SECRET_ARGS[@]}" \
        --dry-run=client -o yaml | kubectl apply -f -

    echo "TLS secret '${RELEASE}-milvus-tls' created in namespace '${NAMESPACE}'"
    if [[ -n "$TLS_CA" ]]; then
        echo "CA certificate (use as MILVUS_CA_CERT):"
        cat "$TLS_CA"
    fi

    ca_pem_path=""
    if [[ -n "$TLS_CA" ]]; then
        ca_pem_path="      caPemPath: /certs/ca.pem"
    fi

    cat > "${CERT_DIR}/values-tls.yaml" <<EOF
extraConfigFiles:
  user.yaml: |+
    proxy:
      http:
        # REST and external gRPC cannot share a port when TLS is enabled.
        port: 8080
    tls:
      serverPemPath: /certs/server.pem
      serverKeyPath: /certs/server.key
${ca_pem_path}
    common:
      security:
        tlsMode: 1
service:
  port: 8080
volumes:
  - name: tls-certs
    secret:
      secretName: ${RELEASE}-milvus-tls
volumeMounts:
  - name: tls-certs
    mountPath: /certs
    readOnly: true
EOF

    TLS_OPTS=(-f "${CERT_DIR}/values-tls.yaml")
fi

# Detect OpenShift vs vanilla Kubernetes
SECURITY_OPTS=()
if kubectl api-resources --api-group=route.openshift.io 2>/dev/null | grep -q routes; then
    echo "Detected OpenShift — clearing hardcoded UIDs for SCC compatibility"
    SECURITY_OPTS=(
        --set etcd.containerSecurityContext.runAsUser=null
        --set etcd.containerSecurityContext.runAsNonRoot=true
        --set etcd.podSecurityContext.fsGroup=null
        --set minio.podSecurityContext.fsGroup=null
        --set minio.containerSecurityContext.runAsUser=null
        --set minio.containerSecurityContext.runAsNonRoot=true
    )
fi

helm repo add milvus https://zilliztech.github.io/milvus-helm/ >/dev/null 2>&1 || true
helm repo update milvus >/dev/null 2>&1

if helm status "$RELEASE" -n "$NAMESPACE" >/dev/null 2>&1; then
    echo "Milvus Helm release '${RELEASE}' already exists in namespace '${NAMESPACE}'"
else
    echo "Installing Milvus standalone via Helm (namespace=${NAMESPACE}, release=${RELEASE}, chart=${CHART_VERSION})"
    helm_install_args=(
        "$RELEASE" milvus/milvus
        -n "$NAMESPACE"
        --version "$CHART_VERSION"
        --set cluster.enabled=false
        --set streaming.messageQueue=rocksmq
        --set pulsarv3.enabled=false
        --set etcd.replicaCount=1
        --set minio.mode=standalone
        --set minio.image.repository=quay.io/minio/minio
        --set minio.image.tag=RELEASE.2025-04-03T14-56-28Z
        --set minio.resources.requests.memory=512Mi
        --set standalone.resources.requests.memory=512Mi
        --set standalone.resources.requests.cpu=200m
    )
    if ((${#SECURITY_OPTS[@]})); then
        helm_install_args+=("${SECURITY_OPTS[@]}")
    fi
    if ((${#TLS_OPTS[@]})); then
        helm_install_args+=("${TLS_OPTS[@]}")
    fi
    helm_install_args+=(--wait "--timeout=$TIMEOUT")

    helm install "${helm_install_args[@]}" || {
        echo "error: failed to install Milvus in namespace '${NAMESPACE}'" >&2
        exit 1
    }
fi

if [[ "$TLS_ENABLED" == "true" ]]; then
    # The chart hardcodes the first service targetPort to the named 19530
    # container port. In TLS mode REST listens on 8080, so route the service
    # to that port instead.
    kubectl patch service "$RELEASE" -n "$NAMESPACE" --type=json \
        -p='[{"op":"replace","path":"/spec/ports/0/port","value":8080},{"op":"replace","path":"/spec/ports/0/targetPort","value":8080}]' \
        >/dev/null || {
        echo "error: failed to route Milvus TLS REST service to port 8080" >&2
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
if [[ "$TLS_ENABLED" == "true" ]]; then
    echo "TLS is enabled — use https:// in MILVUS_URI and set MILVUS_CA_CERT to the CA certificate above"
fi
