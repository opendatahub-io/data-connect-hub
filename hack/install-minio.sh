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
#   hack/install-minio.sh -n dch-tenant -p secretpass --ssl
#
# With user-provided certificates:
#   hack/install-minio.sh \
#       -n dch-tenant \
#       -p secretpass \
#       --ssl \
#       --ssl-cert server.crt \
#       --ssl-key server.key \
#       --ssl-ca ca.crt
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
#   --ssl            enable SSL/TLS and require TLS
#   --ssl-cert FILE  server certificate (PEM)
#   --ssl-key FILE   server private key (PEM)
#   --ssl-ca FILE    CA certificate (PEM)
#   -h, --help       show this help
#
# SSL:
#   --ssl
#       Enable MinIO SSL/TLS and REQUIRE TLS for all TCP connections.
#
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

NAMESPACE="minio"
RELEASE="minio"
USERNAME="minioadmin"
# NOTE: no default password — the caller MUST supply -p.
PASSWORD=""
BUCKET=""
IMAGE="quay.io/minio/minio:latest"
MC_IMAGE=""
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
  -n NAMESPACE     target namespace           (default: minio)
  -r RELEASE       release / resource name    (default: minio)
  -u USER          root user name             (default: minioadmin)
  -p PASSWORD      root password              (required)
  -b BUCKET        bucket to create           (optional)
  -i IMAGE         MinIO server image         (default: quay.io/minio/minio:latest)
  -m MC_IMAGE      MinIO client image         (required if -b is specified)
  -t TIMEOUT       rollout timeout            (default: 300s)
  --ssl            enable SSL/TLS and require TLS
  --ssl-cert FILE  server certificate (PEM)
  --ssl-key FILE   server private key (PEM)
  --ssl-ca FILE    CA certificate (PEM)
  -h, --help       show this help

SSL:
  --ssl
      Enable MinIO SSL/TLS and REQUIRE TLS for all TCP connections.

  --ssl without --ssl-cert/--ssl-key
      Automatically generates a self-signed CA and server certificate.

  --ssl-cert + --ssl-key
      Use a user-provided server certificate and private key.

  --ssl-ca
      Optional CA certificate for client-side server certificate verification.

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
        -m)            require_arg "$@"; MC_IMAGE="$2"; shift 2 ;;
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

if [[ -n "$BUCKET" && -z "$MC_IMAGE" ]]; then
    echo "error: -m MC_IMAGE is required when -b BUCKET is specified" >&2
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
MINIO_SCHEME="http"
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
            -subj "/CN=MinIO Test CA" \
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

    MINIO_SCHEME="https"
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

# ---------------------------------------------------------------------------
# Deploy MinIO
# ---------------------------------------------------------------------------

echo "Installing MinIO (namespace=${NAMESPACE}, release=${RELEASE})"

generate_manifest() {
    cat <<EOF
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
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
            - --certs-dir
            - /etc/minio/certs
EOF
    fi

    cat <<EOF
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
              scheme: ${PROBE_SCHEME}
            initialDelaySeconds: 5
            periodSeconds: 5
            timeoutSeconds: 3
            failureThreshold: 12
          livenessProbe:
            httpGet:
              path: /minio/health/live
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
              mountPath: /etc/minio/certs
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

    # The one-off bootstrap client uses --insecure for TLS because the
    # generated CA is not installed in the mc image's trust store. The CA is
    # still retained in the Kubernetes secret for normal client verification.
    MC_TLS_ARGS=""
    if [[ "$SSL_ENABLED" == "true" ]]; then
        MC_TLS_ARGS="--insecure"
    fi

    kubectl run "$INIT_POD" -n "$NAMESPACE" \
        --image="$MC_IMAGE" \
        --image-pull-policy=IfNotPresent \
        --restart=Never \
        --command -- sh -c "
            mc ${MC_TLS_ARGS} alias set myminio ${MINIO_SCHEME}://${RELEASE}:9000 '${USERNAME}' '${PASSWORD}'
            mc ${MC_TLS_ARGS} mb myminio/${BUCKET} --ignore-existing
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

HOST="${RELEASE}.${NAMESPACE}.svc"

echo ""
echo "MinIO is ready"
echo "  namespace: ${NAMESPACE}"
echo "  release:   ${RELEASE}"
echo "  endpoint:  ${MINIO_SCHEME}://${HOST}:9000"
echo "  console:   ${MINIO_SCHEME}://${HOST}:9001"
echo "  user:      ${USERNAME}"
echo "  ssl:       ${SSL_ENABLED}"
[[ -n "$BUCKET" ]] && echo "  bucket:    ${BUCKET}"

if [[ "$SSL_ENABLED" == "true" ]]; then
    echo ""
    echo "  TLS is REQUIRED for MinIO API and console connections."

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
