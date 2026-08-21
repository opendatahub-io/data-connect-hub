#!/usr/bin/env bash
# Install a local OCI-compliant registry in Kubernetes for testing.
#
# Deploys the CNCF Distribution registry (registry:2) as a single-pod
# Deployment + ClusterIP Service. If a TLS secret name is provided via
# -s, the registry is configured for HTTPS; otherwise it runs plain HTTP.
#
# Usage:
#   hack/install-oci-registry.sh -n dch-tenant                       # HTTP
#   hack/install-oci-registry.sh -n dch-tenant -s oci-registry-tls   # HTTPS
#
# Environment overrides (command-line flags take precedence):
#   OCI_REGISTRY_NAMESPACE      target namespace         (default: dch)
#   OCI_REGISTRY_NAME           deployment/service name   (default: oci-registry)
#   OCI_REGISTRY_IMAGE          registry image            (default: docker.io/library/registry:2)
#   OCI_REGISTRY_PORT           service port              (default: 5000)
#   OCI_REGISTRY_WAIT_TIMEOUT   kubectl wait timeout      (default: 120s)

set -euo pipefail

NAMESPACE="${OCI_REGISTRY_NAMESPACE:-dch}"
NAME="${OCI_REGISTRY_NAME:-oci-registry}"
IMAGE="${OCI_REGISTRY_IMAGE:-docker.io/library/registry:2}"
PORT="${OCI_REGISTRY_PORT:-5000}"
TIMEOUT="${OCI_REGISTRY_WAIT_TIMEOUT:-120s}"
TLS_SECRET=""

usage() {
    echo "Usage: $0 [-n namespace] [-r name] [-s tls-secret] [-i image] [-p port] [-t timeout]"
    exit 1
}

while getopts "n:r:s:i:p:t:h" opt; do
    case $opt in
        n) NAMESPACE="$OPTARG" ;;
        r) NAME="$OPTARG" ;;
        s) TLS_SECRET="$OPTARG" ;;
        i) IMAGE="$OPTARG" ;;
        p) PORT="$OPTARG" ;;
        t) TIMEOUT="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

kubectl create ns "$NAMESPACE" 2>/dev/null || true

# Build container spec fragments based on TLS mode
if [[ -n "$TLS_SECRET" ]]; then
    SCHEME="HTTPS"
    ENV_FRAGMENT=$(cat <<YAML
            - name: REGISTRY_HTTP_TLS_CERTIFICATE
              value: /certs/tls.crt
            - name: REGISTRY_HTTP_TLS_KEY
              value: /certs/tls.key
YAML
    )
    VOLUME_MOUNT_FRAGMENT=$(cat <<YAML
            - name: tls
              mountPath: /certs
              readOnly: true
YAML
    )
    VOLUME_FRAGMENT=$(cat <<YAML
        - name: tls
          secret:
            secretName: ${TLS_SECRET}
YAML
    )
else
    SCHEME="HTTP"
    ENV_FRAGMENT=""
    VOLUME_MOUNT_FRAGMENT=""
    VOLUME_FRAGMENT=""
fi

kubectl apply -n "$NAMESPACE" -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${NAME}
  labels:
    app: ${NAME}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: ${NAME}
  template:
    metadata:
      labels:
        app: ${NAME}
    spec:
      containers:
        - name: registry
          image: ${IMAGE}
          ports:
            - containerPort: 5000
          env:
${ENV_FRAGMENT}
          volumeMounts:
${VOLUME_MOUNT_FRAGMENT}
          readinessProbe:
            httpGet:
              path: /v2/
              port: 5000
              scheme: ${SCHEME}
            initialDelaySeconds: 2
            periodSeconds: 5
          resources:
            requests:
              memory: 64Mi
              cpu: 50m
            limits:
              memory: 256Mi
      volumes:
${VOLUME_FRAGMENT}
---
apiVersion: v1
kind: Service
metadata:
  name: ${NAME}
  labels:
    app: ${NAME}
spec:
  selector:
    app: ${NAME}
  ports:
    - port: ${PORT}
      targetPort: 5000
      protocol: TCP
EOF

kubectl rollout status "deployment/${NAME}" -n "$NAMESPACE" --timeout="$TIMEOUT" || {
    kubectl logs "deployment/${NAME}" -n "$NAMESPACE" --tail=20 || true
    echo "error: OCI registry deployment failed in namespace '${NAMESPACE}'" >&2
    exit 1
}

echo "OCI registry ready: ${NAME}.${NAMESPACE}.svc:${PORT} (${SCHEME})"
