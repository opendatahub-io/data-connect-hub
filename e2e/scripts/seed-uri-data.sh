#!/usr/bin/env bash
# Deploy a lightweight HTTP server serving static JSON for URI connector e2e tests.
#
# Usage:
#   e2e/scripts/seed-uri-data.sh -n <namespace>
#
# Creates:
#   - ConfigMap with JSON test data
#   - nginx Deployment + Service (port 8080)
#
# The service URL is: http://e2e-uri-server.<namespace>.svc:8080

set -euo pipefail

NAMESPACE=""
APP_NAME="e2e-uri-server"

usage() {
    echo "Usage: $0 -n <namespace>"
    exit 1
}

while getopts "n:h" opt; do
    case $opt in
        n) NAMESPACE="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

# --- Detect OpenShift ---

IS_OCP="false"
if kubectl api-resources --api-group=route.openshift.io 2>/dev/null | grep -q routes; then
    IS_OCP="true"
fi

# --- ConfigMap: JSON test data ---

kubectl create configmap "${APP_NAME}-data" \
    -n "$NAMESPACE" \
    --from-literal='cities.json=[{"name":"Tokyo","country":"Japan","population":13960000,"active":true},{"name":"London","country":"United Kingdom","population":8982000,"active":true},{"name":"Paris","country":"France","population":2161000,"active":true},{"name":"New York","country":"United States","population":8336000,"active":true},{"name":"Berlin","country":"Germany","population":3645000,"active":false}]' \
    --from-literal='nested.json={"status":"ok","data":{"items":[{"name":"Tokyo","country":"Japan","population":13960000,"active":true},{"name":"London","country":"United Kingdom","population":8982000,"active":true},{"name":"Paris","country":"France","population":2161000,"active":true},{"name":"New York","country":"United States","population":8336000,"active":true},{"name":"Berlin","country":"Germany","population":3645000,"active":false}]}}' \
    --from-literal='empty.json=[]' \
    --from-literal="binary.dat=binary-test-data-for-e2e
" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# --- ConfigMap: nginx config ---

kubectl create configmap "${APP_NAME}-nginx" \
    -n "$NAMESPACE" \
    --from-literal='default.conf=server {
    listen 8080;
    root /data;
    default_type application/json;
    location /api/ { try_files $uri =404; }
    location /health { return 200 "{\"status\":\"ok\"}"; }
}' \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# --- OCP-specific YAML fragments (writable dirs for nginx) ---

OCP_VOLUME_MOUNTS=""
OCP_VOLUMES=""
OCP_SECURITY_CONTEXT=""
if [[ "$IS_OCP" == "true" ]]; then
    OCP_VOLUME_MOUNTS=$(cat <<'EOF'
            - name: cache
              mountPath: /var/cache/nginx
            - name: tmp
              mountPath: /tmp
            - name: pid
              mountPath: /var/run
EOF
    )
    OCP_VOLUMES=$(cat <<'EOF'
        - name: cache
          emptyDir: {}
        - name: tmp
          emptyDir: {}
        - name: pid
          emptyDir: {}
EOF
    )
    OCP_SECURITY_CONTEXT=$(cat <<'EOF'
          securityContext:
            allowPrivilegeEscalation: false
            runAsNonRoot: true
            seccompProfile:
              type: RuntimeDefault
            capabilities:
              drop: ["ALL"]
EOF
    )
fi

# --- Deployment + Service ---

cat <<EOF | kubectl apply -n "$NAMESPACE" -f - >/dev/null
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ${APP_NAME}
  labels:
    app: ${APP_NAME}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: ${APP_NAME}
  template:
    metadata:
      labels:
        app: ${APP_NAME}
    spec:
      containers:
        - name: nginx
          image: docker.io/library/nginx:alpine
          imagePullPolicy: IfNotPresent
          ports:
            - containerPort: 8080
          volumeMounts:
            - name: data
              mountPath: /data/api
              readOnly: true
            - name: nginx-conf
              mountPath: /etc/nginx/conf.d
              readOnly: true
${OCP_VOLUME_MOUNTS}
${OCP_SECURITY_CONTEXT}
          readinessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 2
            periodSeconds: 5
      volumes:
        - name: data
          configMap:
            name: ${APP_NAME}-data
        - name: nginx-conf
          configMap:
            name: ${APP_NAME}-nginx
${OCP_VOLUMES}
---
apiVersion: v1
kind: Service
metadata:
  name: ${APP_NAME}
  labels:
    app: ${APP_NAME}
spec:
  selector:
    app: ${APP_NAME}
  ports:
    - port: 8080
      targetPort: 8080
      protocol: TCP
EOF

# Restart to pick up any ConfigMap changes, then wait for readiness
kubectl rollout restart deployment/"${APP_NAME}" -n "$NAMESPACE" >/dev/null
kubectl rollout status deployment/"${APP_NAME}" -n "$NAMESPACE" --timeout=60s

echo "URI test server deployed at http://${APP_NAME}.${NAMESPACE}.svc:8080"
