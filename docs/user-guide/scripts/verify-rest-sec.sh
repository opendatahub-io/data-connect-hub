#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
GATEWAY_NS="${2:-${DCH_GATEWAY_NS:-openshift-ingress}}"

echo ""
echo ""
echo "================================== VERIFY REST AUTH ============================="

GW_HOST="dch-gateway-data-science-gateway-class.${GATEWAY_NS}.svc"
CA_CM_NAME="dch-service-ca"
POD_NAME="dch-test-runner"
CA_CERT="/etc/ssl/dch/service-ca.crt"
API_PATH="/api/v1/data/connections"
BASE_URL="https://${GW_HOST}"
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

extract_http_status() {
  grep 'HTTP_STATUS:' | sed 's/.*HTTP_STATUS://' | grep -o '^[0-9]*'
}

run_curl() {
  oc exec "$POD_NAME" -n "$NAMESPACE" -- curl -s --cacert "$CA_CERT" \
    --connect-timeout 5 --max-time 10 "$@" 2>&1
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
      "command": ["sleep", "infinity"],
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
oc wait --for=condition=Ready pod/"$POD_NAME" -n "$NAMESPACE" --timeout=60s

echo "--- Obtaining tokens ---"
SA_ISSUER=$(oc get authentication cluster -o jsonpath='{.spec.serviceAccountIssuer}' 2>/dev/null) || true
if [ -z "$SA_ISSUER" ]; then
  SA_ISSUER="https://kubernetes.default.svc"
fi
echo "  Using audience: $SA_ISSUER"

PROXY_PORT=18081
lsof -ti :$PROXY_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc proxy --port=$PROXY_PORT &>/dev/null &
proxy_pid=$!
sleep 1

get_token() {
  local sa="$1"
  curl -s -X POST "http://127.0.0.1:${PROXY_PORT}/api/v1/namespaces/${NAMESPACE}/serviceaccounts/${sa}/token" \
    -H "Content-Type: application/json" \
    -d "{\"apiVersion\":\"authentication.k8s.io/v1\",\"kind\":\"TokenRequest\",\"spec\":{\"audiences\":[\"${SA_ISSUER}\"],\"expirationSeconds\":3600}}" \
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

echo "--- Testing rest-service via gateway ($GW_HOST) for $NAMESPACE ---"
echo ""

echo "Test 1: Unauthenticated request (expect 401)"
echo "  CMD: curl --cacert $CA_CERT ${BASE_URL}${API_PATH}"
output=$(run_curl -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}${API_PATH}") || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
check_result "No token" \
  "401" \
  "$status (body: $body)"

echo "Test 2: Bad token (expect 401)"
echo "  CMD: curl --cacert $CA_CERT -H 'Authorization: Bearer bad-token' -H 'x-tenant-id: $NAMESPACE' ${BASE_URL}${API_PATH}"
output=$(run_curl -H "Authorization: Bearer bad-token" -H "x-tenant-id: $NAMESPACE" \
  -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}${API_PATH}") || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
check_result "Bad token" \
  "401" \
  "$status (body: $body)"

echo "Test 3: Non-matching path (expect 404)"
echo "  CMD: curl --cacert $CA_CERT ${BASE_URL}/api/v2/data/connections"
output=$(run_curl -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}/api/v2/data/connections") || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
check_result "Non-matching path" \
  "404" \
  "$status (body: $body)"

echo "Test 4: Authenticated request (SA: $SA_NAME)"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: curl --cacert $CA_CERT -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' ${BASE_URL}${API_PATH}"
  output=$(run_curl -H "Authorization: Bearer $user_token" -H "x-tenant-id: $NAMESPACE" \
    -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}${API_PATH}") || true
  status=$(echo "$output" | extract_http_status)
  body=$(echo "$output" | sed '/HTTP_STATUS:/d')
  check_result "Valid token (SA: $SA_NAME)" \
    "200\|400\|501" \
    "$status (body: $body)"
fi

echo "Test 5: Valid token, no RoleBinding (SA: $SA_NOAUTH, expect 403)"
if [ -z "$noauth_token" ]; then
  echo "  SKIPPED  no token for $SA_NOAUTH"
  echo ""
else
  echo "  CMD: curl --cacert $CA_CERT -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' ${BASE_URL}${API_PATH}"
  output=$(run_curl -H "Authorization: Bearer $noauth_token" -H "x-tenant-id: $NAMESPACE" \
    -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}${API_PATH}") || true
  status=$(echo "$output" | extract_http_status)
  body=$(echo "$output" | sed '/HTTP_STATUS:/d')
  check_result "Valid token, no RoleBinding ($SA_NOAUTH)" \
    "403" \
    "$status (body: $body)"
fi

echo "Test 6: TLS verification — skip cacert (expect SSL error)"
echo "  CMD: curl -s (no --cacert) ${BASE_URL}${API_PATH}"
output=$(oc exec "$POD_NAME" -n "$NAMESPACE" -- curl -s --connect-timeout 5 --max-time 10 \
  -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}${API_PATH}" 2>&1) || true
check_result "No CA cert" \
  "exit code 60\|HTTP_STATUS:000" \
  "$output"

echo "Test 7: TLS verification — skip with --insecure (expect 401)"
echo "  CMD: curl -s --insecure ${BASE_URL}${API_PATH}"
output=$(oc exec "$POD_NAME" -n "$NAMESPACE" -- curl -s --insecure --connect-timeout 5 --max-time 10 \
  -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}${API_PATH}" 2>&1) || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
check_result "Skip TLS verification" \
  "401" \
  "$status (body: $body)"

echo "---"
if [ "$failed_count" -eq 0 ]; then
  echo "ALL PASSED: REST auth tests for $NAMESPACE"
else
  echo "FAILED: $failed_count test(s) failed for $NAMESPACE"
  exit 1
fi
