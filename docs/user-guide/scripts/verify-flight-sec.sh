#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
GATEWAY_NS="${2:-${DCH_GATEWAY_NS:-openshift-ingress}}"

echo ""
echo ""
echo "================================== VERIFY FLIGHT AUTH ============================="

GW_HOST="dch-gateway-data-science-gateway-class.${GATEWAY_NS}.svc"
CA_CM_NAME="dch-service-ca"
POD_NAME="dch-test-runner"
CA_CERT="/etc/ssl/dch/service-ca.crt"
SA_NAME="${SA_NAME:-dch-test-user}"
SA_NOAUTH="${SA_NOAUTH:-dch-test-noauth}"

failed_count=0

check_result() {
  local test_name="$1"
  local expected="$2"
  local actual="$3"

  if echo "$actual" | grep -q "$expected"; then
    echo "  expected: \"$expected\""
    echo "  received: \"$actual\""
    echo "  PASSED  "
  else
    echo "  expected: \"$expected\""
    echo "  received: \"$actual\""
    echo "  FAILED  "
    failed_count=$((failed_count + 1))
  fi
  echo ""
}

grpc_method="arrow.flight.protocol.FlightService/ListFlights"
grpc_opts="-cacert $CA_CERT -import-path /tmp -proto Flight.proto"

run_grpcurl() {
  oc exec "$POD_NAME" -n "$NAMESPACE" -- grpcurl $grpc_opts "$@" "$GW_HOST:443" "$grpc_method" 2>&1
}

echo "--- Setup ---"

echo "  Creating CA bundle ConfigMap..."
oc apply -f - <<EOF
apiVersion: v1
kind: ConfigMap
metadata:
  name: ${CA_CM_NAME}
  namespace: $NAMESPACE
  annotations:
    service.beta.openshift.io/inject-cabundle: "true"
EOF

echo "  Waiting for CA bundle injection..."
ca_data=""
for i in $(seq 1 15); do
  ca_data=$(oc get configmap "$CA_CM_NAME" -n "$NAMESPACE" -o jsonpath='{.data.service-ca\.crt}' 2>/dev/null) || true
  if [ -n "$ca_data" ]; then break; fi
  sleep 1
done
if [ -z "$ca_data" ]; then
  echo "FAILED: CA bundle not injected"
  exit 1
fi
echo "  CA bundle ready"

echo "  Creating test runner pod..."
oc get pod "$POD_NAME" -n "$NAMESPACE" &>/dev/null || \
oc run "$POD_NAME" -n "$NAMESPACE" \
  --image=registry.access.redhat.com/ubi9/ubi:latest \
  --restart=Never \
  --overrides="$(cat <<EOJSON
{
  "spec": {
    "containers": [{
      "name": "$POD_NAME",
      "image": "registry.access.redhat.com/ubi9/ubi:latest",
      "command": ["bash", "-c",
        "ARCH=\$(uname -m); if [ \"\$ARCH\" = \"x86_64\" ]; then GA=linux_x86_64; elif [ \"\$ARCH\" = \"aarch64\" ]; then GA=linux_arm64; fi; curl -sL https://github.com/fullstorydev/grpcurl/releases/download/v1.9.1/grpcurl_1.9.1_\${GA}.tar.gz | tar xz -C /usr/local/bin grpcurl && curl -sL -o /tmp/Flight.proto https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto && sleep infinity"
      ],
      "volumeMounts": [
        {"name": "ca-bundle", "mountPath": "/etc/ssl/dch", "readOnly": true}
      ]
    }],
    "volumes": [
      {"name": "ca-bundle", "configMap": {"name": "$CA_CM_NAME"}}
    ],
    "restartPolicy": "Never"
  }
}
EOJSON
)"

echo "  Waiting for test runner pod..."
oc wait --for=condition=Ready pod/"$POD_NAME" -n "$NAMESPACE" --timeout=120s

echo "  Ensuring grpcurl and Flight.proto are available..."
if ! oc exec "$POD_NAME" -n "$NAMESPACE" -- grpcurl --version &>/dev/null; then
  echo "  Installing grpcurl..."
  oc exec "$POD_NAME" -n "$NAMESPACE" -- bash -c '
    ARCH=$(uname -m)
    if [ "$ARCH" = "x86_64" ]; then GA=linux_x86_64; elif [ "$ARCH" = "aarch64" ]; then GA=linux_arm64; fi
    curl -sL "https://github.com/fullstorydev/grpcurl/releases/download/v1.9.1/grpcurl_1.9.1_${GA}.tar.gz" | tar xz -C /usr/local/bin grpcurl
  '
fi
if ! oc exec "$POD_NAME" -n "$NAMESPACE" -- test -f /tmp/Flight.proto; then
  echo "  Downloading Flight.proto..."
  oc exec "$POD_NAME" -n "$NAMESPACE" -- curl -sL -o /tmp/Flight.proto \
    "https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto"
fi
oc exec "$POD_NAME" -n "$NAMESPACE" -- grpcurl --version 2>&1 || true

echo "--- Obtaining tokens ---"
PROXY_PORT=18081
lsof -ti :$PROXY_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc proxy --port=$PROXY_PORT &>/dev/null &
proxy_pid=$!
sleep 1

get_token() {
  local sa="$1"
  curl -s -X POST "http://127.0.0.1:${PROXY_PORT}/api/v1/namespaces/${NAMESPACE}/serviceaccounts/${sa}/token" \
    -H "Content-Type: application/json" \
    -d "{\"apiVersion\":\"authentication.k8s.io/v1\",\"kind\":\"TokenRequest\",\"spec\":{\"audiences\":[\"https://kubernetes.default.svc\"],\"expirationSeconds\":3600}}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['status']['token'])" 2>/dev/null
}

user_token=$(get_token "$SA_NAME") || true
noauth_token=$(get_token "$SA_NOAUTH") || true

kill $proxy_pid 2>/dev/null || true
wait $proxy_pid 2>/dev/null || true

if [ -z "$user_token" ]; then
  echo "  WARNING: could not obtain token for $SA_NAME (run create-test-users.sh first)"
fi
if [ -z "$noauth_token" ]; then
  echo "  WARNING: could not obtain token for $SA_NOAUTH (run create-test-users.sh first)"
fi
echo ""

echo "--- Testing flight-service via gateway ($GW_HOST) for $NAMESPACE ---"
echo ""

echo "Test 1: Unauthenticated gRPC request (expect missing bearer token)"
echo "  CMD: grpcurl $grpc_opts -d '{}' $GW_HOST:443 $grpc_method"
unauth_output=$(run_grpcurl -d '{}') || true
check_result "No token" \
  "missing bearer token" \
  "$unauth_output"

echo "Test 2: Bad token (expect invalid bearer token)"
echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer bad-token' -H 'x-tenant-id: $NAMESPACE' -d '{}' $GW_HOST:443 $grpc_method"
bad_token_output=$(run_grpcurl -H "Authorization: Bearer bad-token" -H "x-tenant-id: $NAMESPACE" -d '{}') || true
check_result "Bad token" \
  "invalid bearer token" \
  "$bad_token_output"

echo "Test 3: Valid token but missing x-tenant-id (expect missing x-tenant-id)"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -d '{}' $GW_HOST:443 $grpc_method"
  missing_tenant_output=$(run_grpcurl -H "Authorization: Bearer $user_token" -d '{}') || true
  check_result "Valid token, no x-tenant-id" \
    "missing x-tenant-id" \
    "$missing_tenant_output"
fi

echo "Test 4: Valid token but wrong x-tenant-id (expect PermissionDenied)"
WRONG_TENANT="default"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $WRONG_TENANT' -d '{}' $GW_HOST:443 $grpc_method"
  wrong_tenant_output=$(run_grpcurl -H "Authorization: Bearer $user_token" -H "x-tenant-id: $WRONG_TENANT" -d '{}') || true
  check_result "Valid token, wrong x-tenant-id (default)" \
    "PermissionDenied" \
    "$wrong_tenant_output"
fi

echo "Test 5: Valid token, no RoleBinding (SA: $SA_NOAUTH, expect PermissionDenied)"
if [ -z "$noauth_token" ]; then
  echo "  SKIPPED  no token for $SA_NOAUTH"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' -d '{}' $GW_HOST:443 $grpc_method"
  noauth_output=$(run_grpcurl -H "Authorization: Bearer $noauth_token" -H "x-tenant-id: $NAMESPACE" -d '{}') || true
  check_result "Valid token, no RoleBinding ($SA_NOAUTH)" \
    "PermissionDenied" \
    "$noauth_output"
fi

echo "Test 6: Authenticated ListFlights (SA: $SA_NAME)"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' -d '{}' $GW_HOST:443 $grpc_method"
  auth_output=$(run_grpcurl -H "Authorization: Bearer $user_token" -H "x-tenant-id: $NAMESPACE" -d '{}') || true
  check_result "Valid token (SA: $SA_NAME)" \
    "Unimplemented\|ListFlights" \
    "$auth_output"
fi

echo "Test 7: TLS verification — no cacert (expect TLS error)"
echo "  CMD: grpcurl -import-path /tmp -proto Flight.proto -d '{}' $GW_HOST:443 $grpc_method"
no_ca_output=$(oc exec "$POD_NAME" -n "$NAMESPACE" -- grpcurl \
  -import-path /tmp -proto Flight.proto -d '{}' "$GW_HOST:443" "$grpc_method" 2>&1) || true
check_result "No CA cert" \
  "certificate\|tls\|x509\|transport" \
  "$no_ca_output"

echo "Test 8: TLS verification — skip with -insecure (expect missing bearer token)"
echo "  CMD: grpcurl -insecure -import-path /tmp -proto Flight.proto -d '{}' $GW_HOST:443 $grpc_method"
insecure_output=$(oc exec "$POD_NAME" -n "$NAMESPACE" -- grpcurl \
  -insecure -import-path /tmp -proto Flight.proto -d '{}' "$GW_HOST:443" "$grpc_method" 2>&1) || true
check_result "Skip TLS verification" \
  "missing bearer token" \
  "$insecure_output"

echo "---"
if [ "$failed_count" -eq 0 ]; then
  echo "ALL PASSED: flight auth tests for $NAMESPACE"
else
  echo "FAILED: $failed_count test(s) failed for $NAMESPACE"
  exit 1
fi
