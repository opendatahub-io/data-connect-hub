#!/usr/bin/env bash
# Install a fixed HTTPS nginx server and seed test data for URI e2e tests.
# Serves JSON, CSV, JSONL, Parquet, and binary files with correct Content-Types.
#
# Usage:
#   e2e/scripts/seed-uri-data.sh -n <namespace> [-r <release>]
#
# Requires: kubectl, openssl, python3 with pyarrow (for Parquet generation).
# The script generates a short-lived test CA and server certificate. The CA is
# stored in <release>-tls-ca/ca.crt for the URI connector credential secret.

set -euo pipefail

NAMESPACE=""
RELEASE="dch-e2e-uri-server"
IMAGE="docker.io/library/nginx:alpine@sha256:72ba65eb42c10344912a84ff42408db7d34f2feb642204570ab8fc5ffd29f1d3"
TIMEOUT="60s"

usage() {
    cat <<USAGE
Usage: $0 -n <namespace> [-r <release>] [-i <image>] [-t <timeout>]

Options:
  -n NAMESPACE     target namespace (required)
  -r RELEASE       resource name (default: dch-e2e-uri-server)
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

PYTHON="${PYTHON:-python3}"
"$PYTHON" -c "import pyarrow" 2>/dev/null || { echo "error: pyarrow not found; set PYTHON to a venv python that has pyarrow" >&2; exit 1; }

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

echo "Generating Parquet test data..."
"$PYTHON" - "$CERT_TMPDIR/cities.parquet" <<'PY' || { echo "error: failed to generate parquet data" >&2; exit 1; }
import sys
import pyarrow as pa, pyarrow.parquet as pq
table = pa.table({
    'active': [True, True, True, True, False],
    'country': ['Japan', 'United Kingdom', 'France', 'United States', 'Germany'],
    'name': ['Tokyo', 'London', 'Paris', 'New York', 'Berlin'],
    'population': pa.array([13960000, 8982000, 2161000, 8336000, 3645000], type=pa.int64()),
})
pq.write_table(table, sys.argv[1])
PY

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
    --from-literal='cities.csv=name,country,population,active
Tokyo,Japan,13960000,true
London,United Kingdom,8982000,true
Paris,France,2161000,true
New York,United States,8336000,true
Berlin,Germany,3645000,false' \
    --from-literal='cities.jsonl={"name":"Tokyo","country":"Japan","population":13960000,"active":true}
{"name":"London","country":"United Kingdom","population":8982000,"active":true}
{"name":"Paris","country":"France","population":2161000,"active":true}
{"name":"New York","country":"United States","population":8336000,"active":true}
{"name":"Berlin","country":"Germany","population":3645000,"active":false}' \
    --from-file="cities.parquet=$CERT_TMPDIR/cities.parquet" \
    --from-file="binary.dat=$CERT_TMPDIR/binary.dat" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

NGINX_CONFIG=$(cat <<EOF
types {
    application/json              json;
    text/csv                      csv;
    application/x-ndjson          jsonl;
    application/vnd.apache.parquet parquet;
    application/octet-stream      bin dat;
}

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

# Use OpenShift's namespace-assigned base UID when available; fall back to the
# nginx image user's UID on non-OpenShift clusters where kubelet needs a
# numeric UID to verify runAsNonRoot (nginx:alpine runs as root by default).
RUN_AS_USER=$(kubectl get namespace "$NAMESPACE" \
    -o jsonpath='{.metadata.annotations.openshift\.io/sa\.scc\.uid-range}' 2>/dev/null \
    | cut -d/ -f1)
: "${RUN_AS_USER:=101}"

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
      # OpenShift namespaces enforce the PodSecurity "restricted" profile, which
      # rejects workloads that do not set an explicit securityContext.
      securityContext:
        runAsNonRoot: true
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: nginx
          image: $IMAGE
          imagePullPolicy: IfNotPresent
          securityContext:
            allowPrivilegeEscalation: false
            capabilities:
              drop: ["ALL"]
            runAsNonRoot: true
            runAsUser: ${RUN_AS_USER}
            seccompProfile:
              type: RuntimeDefault
          ports:
            - containerPort: 8443
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
            # nginx runs as an arbitrary non-root UID, so give it writable
            # paths for its cache, pid file and temp files.
            - name: cache
              mountPath: /var/cache/nginx
            - name: tmp
              mountPath: /tmp
            - name: pid
              mountPath: /var/run
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
        - name: cache
          emptyDir: {}
        - name: tmp
          emptyDir: {}
        - name: pid
          emptyDir: {}
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
