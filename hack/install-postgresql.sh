#!/usr/bin/env bash
#
# Install PostgreSQL using a simple Kubernetes Deployment + Service.
# Optionally enables SSL/TLS.
#
# When SSL is enabled, PostgreSQL is configured to REQUIRE TLS for
# all TCP connections. Plain-text TCP connections are rejected.
#
# The database is intentionally ephemeral (emptyDir), making this suitable
# for E2E/integration tests.
#
# Usage:
#   hack/install-postgresql.sh
#   hack/install-postgresql.sh -n test -r my-pg
#   hack/install-postgresql.sh -n test -r my-pg --ssl
#
# With user-provided certificates:
#   hack/install-postgresql.sh \
#       -n test \
#       -r my-pg \
#       --ssl \
#       --ssl-cert server.crt \
#       --ssl-key server.key \
#       --ssl-ca ca.crt
#

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------

NAMESPACE="postgresql"
RELEASE="postgresql"
CHART_VERSION=""

USERNAME="testuser"
# NOTE: no default password. The operator MUST supply -p, otherwise the
# script exits. A well-known default (e.g. "testpassword") would be a
# hard-coded credential (CWE-798) an attacker could use to log in.
PASSWORD=""
DATABASE="testdb"

IMAGE="docker.io/library/postgres:16"
TIMEOUT="300s"

SSL_ENABLED=false
SSL_CERT=""
SSL_KEY=""
SSL_CA=""

# ---------------------------------------------------------------------------
# Usage
# ---------------------------------------------------------------------------

usage() {
    cat <<USAGE
Usage: $0 [OPTIONS]

Options:
  -n NAMESPACE        target namespace            (default: postgresql)
  -r RELEASE          release name                (default: postgresql)
  -v VERSION          chart version (ignored)
  -u USERNAME         database user               (default: testuser)
  -p PASSWORD         user password               (required)
  -d DATABASE         database name               (default: testdb)
  -i IMAGE            postgres image              (default: docker.io/library/postgres:16)
  -t TIMEOUT          rollout timeout             (default: 300s)
  -s, --ssl           enable SSL/TLS and require TLS
  --ssl-cert FILE     server certificate (PEM)
  --ssl-key FILE      server private key (PEM)
  --ssl-ca FILE       CA certificate (PEM)
  -h, --help          show this help

SSL:
  --ssl
      Enable PostgreSQL SSL/TLS and REQUIRE TLS for all TCP connections.

  --ssl without --ssl-cert/--ssl-key
      Automatically generates a self-signed CA and server certificate.

  --ssl-cert + --ssl-key
      Use a user-provided server certificate and private key.

  --ssl-ca
      Optional CA certificate for client-side server certificate verification.

Examples:

  $0

  $0 -n test -r my-pg

  $0 -n test -r my-pg --ssl

  $0 -n test -r my-pg --ssl \\
      --ssl-cert server.crt \\
      --ssl-key server.key \\
      --ssl-ca ca.crt

USAGE
    exit 1
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: $1 requires an argument" >&2
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        -n)
            require_arg "$@"
            NAMESPACE="$2"
            shift 2
            ;;
        -r)
            require_arg "$@"
            RELEASE="$2"
            shift 2
            ;;
        -v)
            require_arg "$@"
            CHART_VERSION="$2"
            shift 2
            ;;
        -u)
            require_arg "$@"
            USERNAME="$2"
            shift 2
            ;;
        -p)
            require_arg "$@"
            PASSWORD="$2"
            shift 2
            ;;
        -d)
            require_arg "$@"
            DATABASE="$2"
            shift 2
            ;;
        -i)
            require_arg "$@"
            IMAGE="$2"
            shift 2
            ;;
        -t)
            require_arg "$@"
            TIMEOUT="$2"
            shift 2
            ;;
        -s|--ssl)
            SSL_ENABLED=true
            shift
            ;;
        --ssl-cert)
            require_arg "$@"
            SSL_CERT="$2"
            shift 2
            ;;
        --ssl-key)
            require_arg "$@"
            SSL_KEY="$2"
            shift 2
            ;;
        --ssl-ca)
            require_arg "$@"
            SSL_CA="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage
            ;;
    esac
done

if [[ -z "${PASSWORD:-}" ]]; then
    echo "error: password is required; supply it with -p" >&2
    usage
fi

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------

command -v kubectl >/dev/null || {
    echo "error: kubectl not found" >&2
    exit 1
}

if [[ -n "$SSL_CERT" || -n "$SSL_KEY" || -n "$SSL_CA" ]]; then
    SSL_ENABLED=true
fi

if [[ "$SSL_ENABLED" == "true" ]]; then
    command -v openssl >/dev/null || {
        echo "error: openssl not found (required for --ssl)" >&2
        exit 1
    }

    if [[ -n "$SSL_CERT" && -z "$SSL_KEY" ]] ||
       [[ -z "$SSL_CERT" && -n "$SSL_KEY" ]]; then
        echo "error: --ssl-cert and --ssl-key must be provided together" >&2
        exit 1
    fi

    if [[ -n "$SSL_CA" && -z "$SSL_CERT" ]]; then
        echo "error: --ssl-ca requires --ssl-cert and --ssl-key" >&2
        exit 1
    fi
fi

# ---------------------------------------------------------------------------
# Namespace
# ---------------------------------------------------------------------------

kubectl create namespace "$NAMESPACE" \
    --dry-run=client \
    -o yaml |
    kubectl apply -f - >/dev/null

# ---------------------------------------------------------------------------
# Chart version
# ---------------------------------------------------------------------------

if [[ -n "$CHART_VERSION" ]]; then
    echo "Note: -v/PG_CHART_VERSION is ignored in simple install mode"
fi

# ---------------------------------------------------------------------------
# Remove old resources
# ---------------------------------------------------------------------------

kubectl delete deployment "$RELEASE" \
    -n "$NAMESPACE" \
    --ignore-not-found >/dev/null 2>&1 || true

kubectl delete statefulset "$RELEASE" \
    -n "$NAMESPACE" \
    --ignore-not-found >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# Authentication Secret
# ---------------------------------------------------------------------------

kubectl create secret generic "${RELEASE}-auth" \
    -n "$NAMESPACE" \
    --from-literal=POSTGRES_USER="$USERNAME" \
    --from-literal=POSTGRES_PASSWORD="$PASSWORD" \
    --from-literal=POSTGRES_DB="$DATABASE" \
    --dry-run=client \
    -o yaml |
    kubectl apply -f - >/dev/null

# ---------------------------------------------------------------------------
# SSL/TLS
# ---------------------------------------------------------------------------

HAS_CA=false
CERT_TMPDIR=""

if [[ "$SSL_ENABLED" == "true" ]]; then

    if [[ -z "$SSL_CERT" ]]; then
        echo "Generating self-signed SSL certificates..."

        CERT_TMPDIR="$(mktemp -d)"

        cleanup() {
            rm -rf "$CERT_TMPDIR"
        }

        trap cleanup EXIT

        SVC_FQDN="${RELEASE}.${NAMESPACE}.svc.cluster.local"

        # Generate CA.
        openssl req \
            -new \
            -x509 \
            -nodes \
            -days 365 \
            -newkey rsa:2048 \
            -keyout "$CERT_TMPDIR/ca.key" \
            -out "$CERT_TMPDIR/ca.crt" \
            -subj "/CN=PostgreSQL Test CA" \
            2>/dev/null

        # Generate server key and CSR.
        openssl req \
            -new \
            -nodes \
            -newkey rsa:2048 \
            -keyout "$CERT_TMPDIR/server.key" \
            -out "$CERT_TMPDIR/server.csr" \
            -subj "/CN=${SVC_FQDN}" \
            2>/dev/null

        # Server certificate SANs.
        SAN_FILE="$CERT_TMPDIR/san.cnf"

        cat > "$SAN_FILE" <<EOF
subjectAltName=DNS:${RELEASE},DNS:${RELEASE}.${NAMESPACE},DNS:${RELEASE}.${NAMESPACE}.svc,DNS:${SVC_FQDN},DNS:localhost,IP:127.0.0.1
EOF

        # Sign server certificate.
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

        HAS_CA=true
    fi

    TLS_ARGS=(
        --from-file=server.crt="$SSL_CERT"
        --from-file=server.key="$SSL_KEY"
    )

    if [[ "$HAS_CA" == "true" ]]; then
        TLS_ARGS+=(
            --from-file=ca.crt="$SSL_CA"
        )
    fi

    kubectl create secret generic "${RELEASE}-tls" \
        -n "$NAMESPACE" \
        "${TLS_ARGS[@]}" \
        --dry-run=client \
        -o yaml |
        kubectl apply -f - >/dev/null

    echo "SSL certificates stored in secret '${RELEASE}-tls'"

    # -----------------------------------------------------------------------
    # PostgreSQL pg_hba.conf
    #
    # IMPORTANT:
    #   hostnossl -> reject
    #   hostssl   -> allow
    #
    # This makes TLS mandatory for TCP connections.
    #
    # "local" is needed because the official postgres image performs
    # initialization using the local Unix socket.
    # -----------------------------------------------------------------------

    kubectl create configmap "${RELEASE}-pg-hba" \
        -n "$NAMESPACE" \
        --from-literal=pg_hba.conf="\
local all all trust
hostnossl all all 0.0.0.0/0 reject
hostnossl all all ::/0 reject
hostssl all all 0.0.0.0/0 scram-sha-256
hostssl all all ::/0 scram-sha-256
" \
        --dry-run=client \
        -o yaml |
        kubectl apply -f - >/dev/null

    echo "PostgreSQL TCP connections will REQUIRE TLS"
fi

# ---------------------------------------------------------------------------
# Generate Kubernetes manifests
# ---------------------------------------------------------------------------

generate_manifest() {

    cat <<EOF
apiVersion: v1
kind: Service
metadata:
  name: ${RELEASE}
  labels:
    app.kubernetes.io/name: postgresql
    app.kubernetes.io/instance: ${RELEASE}
spec:
  ports:
    - name: postgres
      port: 5432
      targetPort: 5432
  selector:
    app.kubernetes.io/name: postgresql
    app.kubernetes.io/instance: ${RELEASE}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${RELEASE}
  labels:
    app.kubernetes.io/name: postgresql
    app.kubernetes.io/instance: ${RELEASE}
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: postgresql
      app.kubernetes.io/instance: ${RELEASE}
  template:
    metadata:
      labels:
        app.kubernetes.io/name: postgresql
        app.kubernetes.io/instance: ${RELEASE}
    spec:
      initContainers:
        - name: init-pgdata
          image: ${IMAGE}
          imagePullPolicy: IfNotPresent
          command:
            - sh
            - -c
            - |
              set -eu

              mkdir -p /var/lib/postgresql/data/pgdata
              chmod 700 /var/lib/postgresql/data/pgdata
          volumeMounts:
            - name: pgdata
              mountPath: /var/lib/postgresql/data
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
        - name: init-tls
          image: ${IMAGE}
          imagePullPolicy: IfNotPresent
          command:
            - sh
            - -c
            - |
              set -eu

              POSTGRES_UID="\$(id -u postgres)"
              POSTGRES_GID="\$(id -g postgres)"

              cp /etc/tls/readonly/server.crt /etc/tls/certs/server.crt
              cp /etc/tls/readonly/server.key /etc/tls/certs/server.key

              chown "\${POSTGRES_UID}:\${POSTGRES_GID}" \
                /etc/tls/certs/server.key

              chmod 644 /etc/tls/certs/server.crt
              chmod 600 /etc/tls/certs/server.key

              if [ -f /etc/tls/readonly/ca.crt ]; then
                cp /etc/tls/readonly/ca.crt /etc/tls/certs/ca.crt
                chmod 644 /etc/tls/certs/ca.crt
              fi
          volumeMounts:
            - name: tls-readonly
              mountPath: /etc/tls/readonly
              readOnly: true
            - name: tls-certs
              mountPath: /etc/tls/certs
EOF
    fi

    cat <<EOF
      containers:
        - name: postgresql
          image: ${IMAGE}
          imagePullPolicy: IfNotPresent
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
          args:
            - postgres
            - -c
            - ssl=on
            - -c
            - ssl_cert_file=/etc/tls/certs/server.crt
            - -c
            - ssl_key_file=/etc/tls/certs/server.key
            - -c
            - hba_file=/etc/postgresql/pg_hba.conf
EOF
    fi

    cat <<EOF
          ports:
            - containerPort: 5432
              name: postgres

          env:
            - name: PGDATA
              value: /var/lib/postgresql/data/pgdata

          envFrom:
            - secretRef:
                name: ${RELEASE}-auth

          resources:
            requests:
              memory: "256Mi"
              cpu: "250m"

          readinessProbe:
            exec:
              command:
                - sh
                - -c
                - pg_isready -U "\$POSTGRES_USER" -d "\$POSTGRES_DB"
            initialDelaySeconds: 10
            periodSeconds: 5
            timeoutSeconds: 3
            failureThreshold: 12

          livenessProbe:
            exec:
              command:
                - sh
                - -c
                - pg_isready -U "\$POSTGRES_USER" -d "\$POSTGRES_DB"
            initialDelaySeconds: 30
            periodSeconds: 10
            timeoutSeconds: 3
            failureThreshold: 6

          volumeMounts:
            - name: pgdata
              mountPath: /var/lib/postgresql/data
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
            - name: tls-certs
              mountPath: /etc/tls/certs
              readOnly: true

            - name: pg-hba
              mountPath: /etc/postgresql/pg_hba.conf
              subPath: pg_hba.conf
              readOnly: true
EOF
    fi

    cat <<EOF
      volumes:
        - name: pgdata
          emptyDir: {}
EOF

    if [[ "$SSL_ENABLED" == "true" ]]; then
        cat <<EOF
        - name: tls-readonly
          secret:
            secretName: ${RELEASE}-tls

        - name: tls-certs
          emptyDir: {}

        - name: pg-hba
          configMap:
            name: ${RELEASE}-pg-hba
EOF
    fi
}

# ---------------------------------------------------------------------------
# Apply manifests
# ---------------------------------------------------------------------------

generate_manifest |
    kubectl apply -n "$NAMESPACE" -f - >/dev/null

# ---------------------------------------------------------------------------
# Wait for rollout
# ---------------------------------------------------------------------------

if ! kubectl rollout status \
    deployment/"$RELEASE" \
    -n "$NAMESPACE" \
    --timeout="$TIMEOUT"; then

    echo ""
    echo "PostgreSQL failed to become Ready."
    echo ""

    kubectl get pods \
        -n "$NAMESPACE" \
        -l "app.kubernetes.io/instance=${RELEASE}" \
        -o wide || true

    echo ""
    kubectl describe pods \
        -n "$NAMESPACE" \
        -l "app.kubernetes.io/instance=${RELEASE}" || true

    echo ""
    echo "Container logs:"
    kubectl logs \
        -n "$NAMESPACE" \
        -l "app.kubernetes.io/instance=${RELEASE}" \
        --all-containers=true \
        --tail=200 || true

    exit 1
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

HOST="${RELEASE}.${NAMESPACE}.svc.cluster.local"

echo ""
echo "PostgreSQL is ready"
echo "  namespace: ${NAMESPACE}"
echo "  release:   ${RELEASE}"
echo "  host:      ${HOST}:5432"
echo "  database:  ${DATABASE}"
echo "  user:      ${USERNAME}"
echo "  ssl:       ${SSL_ENABLED}"

if [[ "$SSL_ENABLED" == "true" ]]; then
    echo ""
    echo "  TLS is REQUIRED for TCP connections."

    echo ""
    echo "  TLS connection:"
    echo "    postgresql://${USERNAME}:${PASSWORD}@${HOST}:5432/${DATABASE}?sslmode=require"

    if [[ "$HAS_CA" == "true" ]]; then
        echo ""
        echo "  Extract CA:"
        echo "    kubectl get secret ${RELEASE}-tls -n ${NAMESPACE} -o jsonpath='{.data.ca\\.crt}' | base64 -d > ca.crt"

        echo ""
        echo "  TLS connection with certificate verification:"
        echo "    postgresql://${USERNAME}:${PASSWORD}@${HOST}:5432/${DATABASE}?sslmode=verify-full&sslrootcert=ca.crt"
    fi
fi
