#!/usr/bin/env bash
# Install PostgreSQL using a simple Kubernetes Deployment + Service and wait
# for it to become ready.
#
# Usage:
#   e2e/scripts/install-postgresql.sh                               # defaults: namespace=postgresql, release=postgresql
#   e2e/scripts/install-postgresql.sh -n dch -r my-pg               # custom namespace and release name
#
# Environment overrides (command-line flags take precedence):
#   PG_NAMESPACE            target namespace         (default: postgresql)
#   PG_HELM_RELEASE         logical release name     (default: postgresql)
#   PG_CHART_VERSION        accepted for compatibility; ignored
#   PG_USERNAME             database user to create  (default: testuser)
#   PG_PASSWORD             user password            (default: testpassword)
#   PG_DATABASE             database to create       (default: testdb)
#   PG_IMAGE                full image reference     (default: docker.io/library/postgres:16)
#   PG_WAIT_TIMEOUT         rollout wait timeout     (default: 300s)

set -euo pipefail

NAMESPACE="${PG_NAMESPACE:-postgresql}"
RELEASE="${PG_HELM_RELEASE:-postgresql}"
CHART_VERSION="${PG_CHART_VERSION:-}"
USERNAME="${PG_USERNAME:-testuser}"
PASSWORD="${PG_PASSWORD:-testpassword}"
DATABASE="${PG_DATABASE:-testdb}"
IMAGE="${PG_IMAGE:-docker.io/library/postgres:16}"
TIMEOUT="${PG_WAIT_TIMEOUT:-300s}"

usage() {
    echo "Usage: $0 [-n namespace] [-r release] [-v chart-version] [-u username] [-p password] [-d database] [-i image] [-t timeout]"
    exit 1
}

while getopts "n:r:v:u:p:d:i:t:h" opt; do
    case $opt in
        n) NAMESPACE="$OPTARG" ;;
        r) RELEASE="$OPTARG" ;;
        v) CHART_VERSION="$OPTARG" ;;
        u) USERNAME="$OPTARG" ;;
        p) PASSWORD="$OPTARG" ;;
        d) DATABASE="$OPTARG" ;;
        i) IMAGE="$OPTARG" ;;
        t) TIMEOUT="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

kubectl create ns "$NAMESPACE" 2>/dev/null || true

if [[ -n "$CHART_VERSION" ]]; then
    echo "Note: PG_CHART_VERSION/-v is ignored in simple install mode"
fi

# Remove old Helm-managed workload if present so Deployment can reuse the same name.
kubectl delete statefulset "$RELEASE" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

kubectl create secret generic "${RELEASE}-auth" \
    -n "$NAMESPACE" \
    --from-literal=POSTGRES_USER="$USERNAME" \
    --from-literal=POSTGRES_PASSWORD="$PASSWORD" \
    --from-literal=POSTGRES_DB="$DATABASE" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

cat <<EOF | kubectl apply -n "$NAMESPACE" -f - >/dev/null
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
      containers:
        - name: postgresql
          image: ${IMAGE}
          imagePullPolicy: IfNotPresent
          ports:
            - containerPort: 5432
              name: postgres
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
      volumes:
        - name: pgdata
          emptyDir: {}
EOF

kubectl rollout status deployment/"$RELEASE" -n "$NAMESPACE" --timeout="$TIMEOUT" || {
    kubectl get pods -n "$NAMESPACE" -l "app.kubernetes.io/instance=${RELEASE}" -o wide || true
    kubectl describe pods -n "$NAMESPACE" -l "app.kubernetes.io/instance=${RELEASE}" || true
    echo "error: PostgreSQL did not become Ready in namespace '${NAMESPACE}'" >&2
    exit 1
}

echo "PostgreSQL is ready (namespace=${NAMESPACE}, release=${RELEASE}, mode=simple)"
echo "  host:  ${RELEASE}.${NAMESPACE}.svc.cluster.local:5432"
echo "  db:    ${DATABASE}"
echo "  user:  ${USERNAME}"
