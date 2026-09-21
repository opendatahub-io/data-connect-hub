#!/usr/bin/env bash
# Install a fixed HTTPS nginx server and seed JSON/binary data for URI e2e tests.
#
# Usage:
#   e2e/scripts/seed-uri-data.sh -n <namespace> [-r <release>]
#
# The script generates a short-lived test CA and server certificate. The CA is
# stored in <release>-tls-ca/ca.crt for the URI connector credential secret.

set -euo pipefail

NAMESPACE=""
RELEASE="e2e-uri-server"
IMAGE="docker.io/library/nginx:alpine@sha256:72ba65eb42c10344912a84ff42408db7d34f2feb642204570ab8fc5ffd29f1d3"
TIMEOUT="60s"

usage() {
    cat <<USAGE
Usage: $0 -n <namespace> [-r <release>] [-i <image>] [-t <timeout>]

Options:
  -n NAMESPACE     target namespace (required)
  -r RELEASE       resource name (default: e2e-uri-server)
  -i IMAGE         nginx image (default: docker.io/library/nginx:alpine)
  -t TIMEOUT       rollout timeout (default: 60s)
  -h               show this help
USAGE
}

while getopts "n:r:i:t:h" opt; do
    case "$opt" in
        n) NAMESPACE="$OPTARG" ;;
        r) RELEASE="$OPTARG" ;;
        i) IMAGE="$OPTARG" ;;
        t) TIMEOUT="$OPTARG" ;;
        h) usage; exit 0 ;;
        *) usage >&2; exit 1 ;;
    esac
done

[[ -n "$NAMESPACE" ]] || { usage >&2; exit 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }
command -v openssl >/dev/null || { echo "error: openssl not found" >&2; exit 1; }

CERT_TMPDIR=$(mktemp -d)
cleanup() {
    rm -rf "$CERT_TMPDIR"
}
trap cleanup EXIT

kubectl create namespace "$NAMESPACE" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

SERVICE_FQDN="$RELEASE.$NAMESPACE.svc.cluster.local"
echo "Generating URI test server CA and certificate..."
openssl req -x509 -nodes -newkey rsa:2048 \
    -keyout "$CERT_TMPDIR/ca.key" \
    -out "$CERT_TMPDIR/ca.crt" \
    -subj "/CN=URI E2E Test CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -days 365 2>/dev/null

openssl req -new -nodes -newkey rsa:2048 \
    -keyout "$CERT_TMPDIR/server.key" \
    -out "$CERT_TMPDIR/server.csr" \
    -subj "/CN=$SERVICE_FQDN" 2>/dev/null

cat > "$CERT_TMPDIR/server-ext.cnf" <<EOF
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:$RELEASE,DNS:$RELEASE.$NAMESPACE,DNS:$RELEASE.$NAMESPACE.svc,DNS:$SERVICE_FQDN,DNS:localhost,IP:127.0.0.1
EOF

openssl x509 -req \
    -in "$CERT_TMPDIR/server.csr" \
    -CA "$CERT_TMPDIR/ca.crt" \
    -CAkey "$CERT_TMPDIR/ca.key" \
    -CAcreateserial \
    -out "$CERT_TMPDIR/server.crt" \
    -days 365 \
    -extfile "$CERT_TMPDIR/server-ext.cnf" 2>/dev/null

printf 'binary-test-data-for-e2e\n' > "$CERT_TMPDIR/binary.dat"

kubectl create secret tls "$RELEASE-tls" \
    -n "$NAMESPACE" \
    --cert="$CERT_TMPDIR/server.crt" \
    --key="$CERT_TMPDIR/server.key" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl create secret generic "$RELEASE-tls-ca" \
    -n "$NAMESPACE" \
    --from-file=ca.crt="$CERT_TMPDIR/ca.crt" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

kubectl create configmap "$RELEASE-data" \
    -n "$NAMESPACE" \
    --from-literal='cities.json=[{"name":"Tokyo","country":"Japan","population":13960000,"active":true},{"name":"London","country":"United Kingdom","population":8982000,"active":true},{"name":"Paris","country":"France","population":2161000,"active":true},{"name":"New York","country":"United States","population":8336000,"active":true},{"name":"Berlin","country":"Germany","population":3645000,"active":false}]' \
    --from-literal='nested.json={"status":"ok","data":{"items":[{"name":"Tokyo","country":"Japan","population":13960000,"active":true},{"name":"London","country":"United Kingdom","population":8982000,"active":true},{"name":"Paris","country":"France","population":2161000,"active":true},{"name":"New York","country":"United States","population":8336000,"active":true},{"name":"Berlin","country":"Germany","population":3645000,"active":false}]}}' \
    --from-literal='empty.json=[]' \
    --from-file="binary.dat=$CERT_TMPDIR/binary.dat" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

NGINX_CONFIG=$(cat <<EOF
server {
    listen 8443 ssl;
    root /data;
    default_type application/json;
    ssl_certificate /etc/nginx/tls/tls.crt;
    ssl_certificate_key /etc/nginx/tls/tls.key;
    ssl_protocols TLSv1.2 TLSv1.3;
    location /api/ { try_files \$uri =404; }
    location /health { return 200 '{"status":"ok"}'; }
}
EOF
)
kubectl create configmap "$RELEASE-nginx" \
    -n "$NAMESPACE" \
    --from-literal="default.conf=$NGINX_CONFIG" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

IS_OCP="false"
if kubectl api-resources --api-group=route.openshift.io 2>/dev/null | grep -q routes; then
    IS_OCP="true"
fi

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

cat <<EOF | kubectl apply -n "$NAMESPACE" -f - >/dev/null
apiVersion: apps/v1
kind: Deployment
metadata:
  name: $RELEASE
  labels:
    app: $RELEASE
spec:
  replicas: 1
  selector:
    matchLabels:
      app: $RELEASE
  template:
    metadata:
      labels:
        app: $RELEASE
    spec:
      containers:
        - name: nginx
          image: $IMAGE
          imagePullPolicy: IfNotPresent
          ports:
            - containerPort: 8443
$OCP_SECURITY_CONTEXT
          volumeMounts:
            - name: data
              mountPath: /data/api
              readOnly: true
            - name: nginx-conf
              mountPath: /etc/nginx/conf.d
              readOnly: true
            - name: tls
              mountPath: /etc/nginx/tls
              readOnly: true
$OCP_VOLUME_MOUNTS
          readinessProbe:
            httpGet:
              scheme: HTTPS
              path: /health
              port: 8443
            initialDelaySeconds: 2
            periodSeconds: 5
      volumes:
        - name: data
          configMap:
            name: $RELEASE-data
        - name: nginx-conf
          configMap:
            name: $RELEASE-nginx
        - name: tls
          secret:
            secretName: $RELEASE-tls
$OCP_VOLUMES
---
apiVersion: v1
kind: Service
metadata:
  name: $RELEASE
  labels:
    app: $RELEASE
spec:
  selector:
    app: $RELEASE
  ports:
    - name: https
      port: 8443
      targetPort: 8443
      protocol: TCP
EOF

kubectl rollout restart deployment/"$RELEASE" -n "$NAMESPACE" >/dev/null
kubectl rollout status deployment/"$RELEASE" -n "$NAMESPACE" --timeout="$TIMEOUT"

echo "URI HTTPS test server deployed at https://$RELEASE.$NAMESPACE.svc:8443"
echo "CA Secret: $RELEASE-tls-ca"
