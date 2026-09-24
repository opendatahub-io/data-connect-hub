#!/usr/bin/env bash
# Install SeaweedFS as a single-node S3-compatible object store.
#
# This script installs the SeaweedFS S3-compatible test backend.
# The data is intentionally ephemeral (emptyDir), making this suitable for
# E2E/integration tests. Requested buckets are created through the S3 API by
# a short-lived AWS CLI pod after SeaweedFS becomes ready.
#
# Usage:
#   hack/install-seaweedfs.sh -n dch-tenant -p secretpass
#   hack/install-seaweedfs.sh -n dch-tenant -p secretpass -b my-bucket
#   hack/install-seaweedfs.sh -n dch-tenant -p secretpass --ssl
#
# Options:
#   -n NAMESPACE     target namespace           (default: seaweedfs)
#   -r RELEASE       release / resource name    (default: seaweedfs)
#   -u USER          S3 access key              (default: s3admin)
#   -p PASSWORD      S3 secret key              (required)
#   -b BUCKET        bucket(s) to create        (optional)
#   -i IMAGE         SeaweedFS image            (default: docker.io/chrislusf/seaweedfs:3.99)
#   -m IMAGE         S3 client image            (default: docker.io/amazon/aws-cli:2.31.0)
#   -t TIMEOUT       rollout timeout            (default: 300s)
#   --ssl            enable SSL/TLS and require TLS
#   --ssl-cert FILE  server certificate (PEM)
#   --ssl-key FILE   server private key (PEM)
#   --ssl-ca FILE    CA certificate (PEM)
#   -h, --help       show this help
#
# SSL:
#   --ssl without --ssl-cert/--ssl-key automatically generates a self-signed
#   CA and server certificate. SeaweedFS serves HTTPS on the S3 port when the
#   certificate and key flags are supplied.
#
set -euo pipefail

NAMESPACE="seaweedfs"
RELEASE="seaweedfs"
USERNAME="s3admin"
# NOTE: no default password — the caller MUST supply -p.
PASSWORD=""
BUCKET=""
IMAGE="docker.io/chrislusf/seaweedfs:3.99"
CLIENT_IMAGE="docker.io/amazon/aws-cli:2.31.0"
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
    cat <<USAGE
Usage: $0 [OPTIONS]

Options:
  -n NAMESPACE     target namespace           (default: seaweedfs)
  -r RELEASE       release / resource name    (default: seaweedfs)
  -u USER          S3 access key              (default: s3admin)
  -p PASSWORD      S3 secret key              (required)
  -b BUCKET        bucket(s) to create        (optional)
  -i IMAGE         SeaweedFS image            (default: docker.io/chrislusf/seaweedfs:3.99)
  -m IMAGE         S3 client image            (default: docker.io/amazon/aws-cli:2.31.0)
  -t TIMEOUT       rollout timeout            (default: 300s)
  --ssl            enable SSL/TLS and require TLS
  --ssl-cert FILE  server certificate (PEM)
  --ssl-key FILE   server private key (PEM)
  --ssl-ca FILE    CA certificate (PEM)
  -h, --help       show this help

SSL:
  --ssl without --ssl-cert/--ssl-key
      Automatically generates a self-signed CA and server certificate.

  --ssl-cert + --ssl-key
      Use a user-provided server certificate and private key.

Examples:
  $0 -p secretpass
  $0 -n dch-tenant -p secretpass --ssl
  $0 -n dch-tenant -p secretpass --ssl \\
      --ssl-cert server.crt \\
      --ssl-key server.key \\
      --ssl-ca ca.crt
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)            require_arg "$@"; NAMESPACE="$2"; shift 2 ;;
        -r)            require_arg "$@"; RELEASE="$2"; shift 2 ;;
        -u)            require_arg "$@"; USERNAME="$2"; shift 2 ;;
        -p)            require_arg "$@"; PASSWORD="$2"; shift 2 ;;
        -b)            require_arg "$@"; BUCKET="$2"; shift 2 ;;
        -i)            require_arg "$@"; IMAGE="$2"; shift 2 ;;
        -m)            require_arg "$@"; CLIENT_IMAGE="$2"; shift 2 ;;
        -t)            require_arg "$@"; TIMEOUT="$2"; shift 2 ;;
        --ssl)         SSL_ENABLED=true; shift ;;
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
    exit 1
fi

command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

if [[ -n "$SSL_CERT" || -n "$SSL_KEY" || -n "$SSL_CA" ]]; then
    SSL_ENABLED=true
fi

if [[ "$SSL_ENABLED" == "true" ]]; then
    if [[ -n "$SSL_CERT" && -z "$SSL_KEY" ]] ||
       [[ -z "$SSL_CERT" && -n "$SSL_KEY" ]]; then
        echo "error: --ssl-cert and --ssl-key must be provided together" >&2
        exit 1
    fi

    if [[ -n "$SSL_CA" && -z "$SSL_CERT" ]]; then
        echo "error: --ssl-ca requires --ssl-cert and --ssl-key" >&2
        exit 1
    fi

    if [[ -z "$SSL_CERT" ]]; then
        command -v openssl >/dev/null || {
            echo "error: openssl not found (required to generate certificates)" >&2
            exit 1
        }
    fi
fi

# ---------------------------------------------------------------------------
# SSL/TLS
# ---------------------------------------------------------------------------

CERT_TMPDIR=""
S3_SCHEME="http"
PROBE_SCHEME="HTTP"

if [[ "$SSL_ENABLED" == "true" ]]; then
    if [[ -z "$SSL_CERT" ]]; then
        echo "Generating self-signed SSL certificates..."

        CERT_TMPDIR="$(mktemp -d)"

        cleanup() {
            rm -rf "$CERT_TMPDIR"
        }

        trap cleanup EXIT

        SVC_FQDN="${RELEASE}.${NAMESPACE}.svc.cluster.local"

        openssl req \
            -new \
            -x509 \
            -nodes \
            -days 365 \
            -newkey rsa:2048 \
            -keyout "$CERT_TMPDIR/ca.key" \
            -out "$CERT_TMPDIR/ca.crt" \
            -subj "/CN=SeaweedFS Test CA" \
            2>/dev/null

        openssl req \
            -new \
            -nodes \
            -newkey rsa:2048 \
            -keyout "$CERT_TMPDIR/server.key" \
            -out "$CERT_TMPDIR/server.csr" \
            -subj "/CN=${SVC_FQDN}" \
            2>/dev/null

        SAN_FILE="$CERT_TMPDIR/san.cnf"

        cat > "$SAN_FILE" <<EOF
subjectAltName=DNS:${RELEASE},DNS:${RELEASE}.${NAMESPACE},DNS:${RELEASE}.${NAMESPACE}.svc,DNS:${SVC_FQDN},DNS:localhost,IP:127.0.0.1
EOF

        openssl x509 \
            -req \
            -in "$CERT_TMPDIR/server.csr" \
            -CA "$CERT_TMPDIR/ca.crt" \
            -CAkey "$CERT_TMPDIR/ca.key" \
            -CAcreateserial \
            -out "$CERT_TMPDIR/server.crt" \
            -days 365 \
            -extfile "$SAN_FILE" \
            2>/dev/null

        SSL_CERT="$CERT_TMPDIR/server.crt"
        SSL_KEY="$CERT_TMPDIR/server.key"
        SSL_CA="$CERT_TMPDIR/ca.crt"
    fi

    [[ -f "$SSL_CERT" ]] || {
        echo "error: certificate file not found: $SSL_CERT" >&2
        exit 1
    }

    [[ -f "$SSL_KEY" ]] || {
        echo "error: private key file not found: $SSL_KEY" >&2
        exit 1
    }

    if [[ -n "$SSL_CA" ]]; then
        [[ -f "$SSL_CA" ]] || {
            echo "error: CA certificate file not found: $SSL_CA" >&2
            exit 1
        }
    fi

    S3_SCHEME="https"
    PROBE_SCHEME="HTTPS"
fi

# ---------------------------------------------------------------------------
# Namespace
# ---------------------------------------------------------------------------

kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# ---------------------------------------------------------------------------
# TLS Secret
# ---------------------------------------------------------------------------

if [[ "$SSL_ENABLED" == "true" ]]; then
    TLS_ARGS=(
        "--from-file=public.crt=$SSL_CERT"
        "--from-file=private.key=$SSL_KEY"
    )

    if [[ -n "$SSL_CA" ]]; then
        TLS_ARGS+=("--from-file=ca.crt=$SSL_CA")
    fi

    kubectl create secret generic "${RELEASE}-tls" \
        -n "$NAMESPACE" \
        "${TLS_ARGS[@]}" \
        --dry-run=client \
        -o yaml |
        kubectl apply -f - >/dev/null

    echo "SSL certificates stored in secret '${RELEASE}-tls'"
fi

# ---------------------------------------------------------------------------
# Remove old resources
# ---------------------------------------------------------------------------

kubectl delete deployment "$RELEASE" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
# Remove the old console port from an existing Service. SeaweedFS exposes
# one S3 endpoint.
kubectl delete service "$RELEASE" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# Deploy SeaweedFS
# ---------------------------------------------------------------------------

echo "Installing SeaweedFS (namespace=${NAMESPACE}, release=${RELEASE})"

generate_manifest() {
    cat <<EOF
apiVersion: v1
kind: Service
metadata:
  name: ${RELEASE}
  labels:
    app.kubernetes.io/name: seaweedfs
    app.kubernetes.io/instance: ${RELEASE}
spec:
  ports:
    - name: s3
      port: 9000
      targetPort: 9000
  selector:
    app.kubernetes.io/name: seaweedfs
    app.kubernetes.io/instance: ${RELEASE}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${RELEASE}
  labels:
    app.kubernetes.io/name: seaweedfs
    app.kubernetes.io/instance: ${RELEASE}
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: seaweedfs
      app.kubernetes.io/instance: ${RELEASE}
  template:
    metadata:
      labels:
        app.kubernetes.io/name: seaweedfs
        app.kubernetes.io/instance: ${RELEASE}
    spec:
      containers:
        - name: seaweedfs
          image: ${IMAGE}
          imagePullPolicy: IfNotPresent
          args:
            - server
            - -filer
            - -s3
            - -ip.bind=0.0.0.0
            - -s3.port=9000
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
            - -s3.cert.file=/etc/seaweedfs/certs/public.crt
            - -s3.key.file=/etc/seaweedfs/certs/private.key
EOF
    fi

    cat <<EOF
          env:
            - name: AWS_ACCESS_KEY_ID
              value: "${USERNAME}"
            - name: AWS_SECRET_ACCESS_KEY
              value: "${PASSWORD}"
EOF

    cat <<EOF
          ports:
            - containerPort: 9000
              name: s3
          resources:
            requests:
              memory: "256Mi"
              cpu: "250m"
          readinessProbe:
            httpGet:
              path: /healthz
              port: 9000
              scheme: ${PROBE_SCHEME}
            initialDelaySeconds: 5
            periodSeconds: 5
            timeoutSeconds: 3
            failureThreshold: 12
          livenessProbe:
            httpGet:
              path: /healthz
              port: 9000
              scheme: ${PROBE_SCHEME}
            initialDelaySeconds: 10
            periodSeconds: 10
            timeoutSeconds: 3
            failureThreshold: 6
          volumeMounts:
            - name: data
              mountPath: /data
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
            - name: tls-certs
              mountPath: /etc/seaweedfs/certs
              readOnly: true
EOF
    fi

    cat <<EOF
      volumes:
        - name: data
          emptyDir: {}
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
        - name: tls-certs
          secret:
            secretName: ${RELEASE}-tls
            items:
              - key: public.crt
                path: public.crt
              - key: private.key
                path: private.key
EOF
    fi
}

generate_manifest | kubectl apply -n "$NAMESPACE" -f - >/dev/null

# ---------------------------------------------------------------------------
# Wait for rollout
# ---------------------------------------------------------------------------

if ! kubectl rollout status deployment/"$RELEASE" -n "$NAMESPACE" --timeout="$TIMEOUT"; then
    echo ""
    echo "SeaweedFS failed to become Ready."
    kubectl get pods -n "$NAMESPACE" -l "app.kubernetes.io/instance=${RELEASE}" -o wide || true
    echo ""
    kubectl logs -n "$NAMESPACE" -l "app.kubernetes.io/instance=${RELEASE}" --tail=50 || true
    exit 1
fi

# ---------------------------------------------------------------------------
# Create buckets (optional)
# ---------------------------------------------------------------------------

if [[ -n "$BUCKET" ]]; then
    echo "Creating S3 bucket(s) '${BUCKET}'"

    INIT_POD="${RELEASE}-init-bucket"
    kubectl delete pod "$INIT_POD" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

    S3_CLIENT_TLS_ARGS=""
    if [[ "$SSL_ENABLED" == "true" ]]; then
        # The generated CA is not installed in the AWS CLI image. The CA is
        # retained in the Kubernetes secret for normal client verification.
        S3_CLIENT_TLS_ARGS="--no-verify-ssl"
    fi

    kubectl run "$INIT_POD" -n "$NAMESPACE" \
        --image="$CLIENT_IMAGE" \
        --image-pull-policy=IfNotPresent \
        --restart=Never \
        --env="AWS_ACCESS_KEY_ID=${USERNAME}" \
        --env="AWS_SECRET_ACCESS_KEY=${PASSWORD}" \
        --env="AWS_DEFAULT_REGION=us-east-1" \
        --env="AWS_S3_ADDRESSING_STYLE=path" \
        --command -- /bin/sh -ceu "
            ready=0
            for i in \$(seq 1 60); do
              if aws ${S3_CLIENT_TLS_ARGS} --endpoint-url '${S3_SCHEME}://${RELEASE}:9000' s3api list-buckets >/dev/null 2>&1; then
                ready=1
                break
              fi
              sleep 2
            done
            [ \"\$ready\" -eq 1 ] || { echo 'S3 endpoint not reachable after retries' >&2; exit 1; }
            for bucket in ${BUCKET//,/ }; do
              aws ${S3_CLIENT_TLS_ARGS} --endpoint-url '${S3_SCHEME}://${RELEASE}:9000' s3api create-bucket --bucket \"\$bucket\" >/dev/null 2>&1 ||
                aws ${S3_CLIENT_TLS_ARGS} --endpoint-url '${S3_SCHEME}://${RELEASE}:9000' s3api head-bucket --bucket \"\$bucket\"
            done
        "

    kubectl wait --for=jsonpath='{.status.phase}'=Succeeded \
        "pod/$INIT_POD" -n "$NAMESPACE" --timeout=120s || {
        kubectl logs "$INIT_POD" -n "$NAMESPACE" --tail=20 || true
        echo "error: S3 bucket creation failed" >&2
        exit 1
    }
    kubectl delete pod "$INIT_POD" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

    echo "S3 bucket(s) '${BUCKET}' created"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

HOST="${RELEASE}.${NAMESPACE}.svc"

echo ""
echo "SeaweedFS is ready"
echo "  namespace: ${NAMESPACE}"
echo "  release:   ${RELEASE}"
echo "  endpoint:  ${S3_SCHEME}://${HOST}:9000"
echo "  user:      ${USERNAME}"
echo "  ssl:       ${SSL_ENABLED}"
[[ -n "$BUCKET" ]] && echo "  bucket(s): ${BUCKET}"

if [[ "$SSL_ENABLED" == "true" ]]; then
    echo ""
    echo "  TLS is REQUIRED for S3 connections."

    if [[ -n "$SSL_CA" ]]; then
        echo ""
        echo "  Extract CA:"
        echo "    kubectl get secret ${RELEASE}-tls -n ${NAMESPACE} -o jsonpath='{.data.ca\\.crt}' | base64 -d > ca.crt"

        echo ""
        echo "  S3 endpoint (certificate verification):"
        echo "    https://${HOST}:9000"
    else
        echo ""
        echo "  S3 endpoint (encryption without certificate verification):"
        echo "    https://${HOST}:9000"
    fi
fi
