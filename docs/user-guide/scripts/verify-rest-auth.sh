#!/bin/bash
set -euo pipefail

MODE="${1:-gateway}"
NAMESPACE="${2:-dch-example}"
GATEWAY_NS="${3:-${DCH_GATEWAY_NS:-openshift-ingress}}"

if [ "$MODE" != "gateway" ] && [ "$MODE" != "pod" ]; then
  echo "Usage: $0 <gateway|pod> [namespace] [gateway-namespace]"
  echo "  gateway  — test via gateway (TLS, port-forward to gateway svc)"
  echo "  pod      — test directly against rest-service pod (plaintext)"
  exit 1
fi

echo ""
echo ""
echo "================================== VERIFY REST AUTH ============================="

pf_pid=""
cleanup() {
  if [ -n "$pf_pid" ]; then
    kill $pf_pid 2>/dev/null || true
    wait $pf_pid 2>/dev/null || true
  fi
}
trap cleanup EXIT

failed_count=0

check_result() {
  local test_name="$1"
  local expected="$2"
  local actual="$3"
  local pattern="$4"

  if echo "$actual" | grep -q "$pattern"; then
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

if [ "$MODE" = "gateway" ]; then
  GW_SVC="dch-gateway-data-science-gateway-class"
  LOCAL_PORT=8443
  SCHEME="https"
  CURL_OPTS="-sk"
  echo "--- Testing rest-service via gateway for $NAMESPACE ---"

  lsof -ti :$LOCAL_PORT 2>/dev/null | xargs kill 2>/dev/null || true
  echo "  Waiting for port $LOCAL_PORT to be free..."
  for i in $(seq 1 10); do
    lsof -ti :$LOCAL_PORT &>/dev/null || break
    sleep 1
  done

  echo "  Port-forwarding $GW_SVC:443 -> localhost:$LOCAL_PORT..."
  oc port-forward "svc/$GW_SVC" -n "$GATEWAY_NS" "$LOCAL_PORT:443" &
  pf_pid=$!

  echo "  Waiting for port-forward..."
  pf_ready=false
  for i in $(seq 1 15); do
    if curl -sk --connect-timeout 1 --max-time 2 -o /dev/null "https://localhost:$LOCAL_PORT" 2>/dev/null; then
      pf_ready=true
      break
    fi
    sleep 1
  done
  if [ "$pf_ready" != "true" ]; then
    echo "  WARNING: port-forward may not be ready, proceeding anyway..."
  fi
else
  LOCAL_PORT=8080
  SCHEME="http"
  CURL_OPTS="-s"
  echo "--- Testing rest-service directly via pod for $NAMESPACE ---"

  rest_pod=$(oc get po -n "$NAMESPACE" -l app.kubernetes.io/name=rest-service -o jsonpath='{.items[0].metadata.name}')

  lsof -ti :$LOCAL_PORT 2>/dev/null | xargs kill 2>/dev/null || true
  echo "  Waiting for port $LOCAL_PORT to be free..."
  for i in $(seq 1 10); do
    lsof -ti :$LOCAL_PORT &>/dev/null || break
    sleep 1
  done

  echo "  Port-forwarding $rest_pod:8080 -> localhost:$LOCAL_PORT..."
  oc port-forward "$rest_pod" -n "$NAMESPACE" "$LOCAL_PORT:8080" &
  pf_pid=$!
  sleep 2
fi

API_PATH="/api/v1/data/connections"
BASE_URL="${SCHEME}://localhost:${LOCAL_PORT}"

echo ""
echo "Test 1: Unauthenticated request"
echo "  CMD: curl $CURL_OPTS ${BASE_URL}${API_PATH}"
output=$(curl $CURL_OPTS --connect-timeout 5 --max-time 10 \
  -w '\nHTTP_STATUS:%{http_code}' \
  "${BASE_URL}${API_PATH}" 2>&1) || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
check_result "No token" \
  "401" \
  "$status (body: $body)" \
  "401"

echo "Test 2: Bad token request"
echo "  CMD: curl $CURL_OPTS -H 'Authorization: Bearer bad-token' -H 'x-tenant-id: $NAMESPACE' ${BASE_URL}${API_PATH}"
output=$(curl $CURL_OPTS --connect-timeout 5 --max-time 10 \
  -H "Authorization: Bearer bad-token" \
  -H "x-tenant-id: $NAMESPACE" \
  -w '\nHTTP_STATUS:%{http_code}' \
  "${BASE_URL}${API_PATH}" 2>&1) || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
check_result "Bad token" \
  "401" \
  "$status (body: $body)" \
  "401"

echo "Test 3: Non-matching path"
echo "  CMD: curl $CURL_OPTS ${BASE_URL}/api/v2/data/connections"
output=$(curl $CURL_OPTS --connect-timeout 5 --max-time 10 \
  -w '\nHTTP_STATUS:%{http_code}' \
  "${BASE_URL}/api/v2/data/connections" 2>&1) || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
check_result "Non-matching path" \
  "404" \
  "$status (body: $body)" \
  "404"

SA_NAME="${SA_NAME:-dch-test-user}"
echo "Test 4: Authenticated request (SA: $SA_NAME)"

SA_ISSUER=$(oc get authentication cluster -o jsonpath='{.spec.serviceAccountIssuer}' 2>/dev/null) || true
if [ -z "$SA_ISSUER" ]; then
  SA_ISSUER="https://kubernetes.default.svc"
fi
echo "  Using audience: $SA_ISSUER"

PROXY_PORT=18081
echo "  Step 1: oc proxy --port=$PROXY_PORT"
lsof -ti :$PROXY_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc proxy --port=$PROXY_PORT &>/dev/null &
proxy_pid=$!
sleep 1

TOKEN_URL="http://127.0.0.1:${PROXY_PORT}/api/v1/namespaces/${NAMESPACE}/serviceaccounts/${SA_NAME}/token"
TOKEN_BODY="{\"apiVersion\":\"authentication.k8s.io/v1\",\"kind\":\"TokenRequest\",\"spec\":{\"audiences\":[\"${SA_ISSUER}\"],\"expirationSeconds\":3600}}"
echo "  Step 2: curl -s -X POST $TOKEN_URL -H 'Content-Type: application/json' -d '$TOKEN_BODY'"

user_token=$(curl -s -X POST "$TOKEN_URL" \
  -H "Content-Type: application/json" \
  -d "$TOKEN_BODY" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['status']['token'])" 2>/dev/null) || true

kill $proxy_pid 2>/dev/null || true
wait $proxy_pid 2>/dev/null || true

if [ -z "$user_token" ]; then
  echo "  SKIPPED  could not obtain token for $SA_NAME (run create-test-users.sh first)"
  echo ""
else
  echo "  CMD: curl $CURL_OPTS -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' ${BASE_URL}${API_PATH}"
  output=$(curl $CURL_OPTS --connect-timeout 5 --max-time 10 \
    -H "Authorization: Bearer $user_token" \
    -H "x-tenant-id: $NAMESPACE" \
    -w '\nHTTP_STATUS:%{http_code}' \
    "${BASE_URL}${API_PATH}" 2>&1) || true
  status=$(echo "$output" | extract_http_status)
  body=$(echo "$output" | sed '/HTTP_STATUS:/d')
  check_result "Valid token (SA: $SA_NAME)" \
    "200, 400, or 501 (auth passed, reached service)" \
    "$status (body: $body)" \
    "200\|400\|501"
fi

ROLE="${ROLE:-dch-ingest}"
BINDING_NAME="${SA_NAME}-${ROLE}"

echo "Test 5: Authorized token but no RoleBinding (expect 403)"
echo "  Step 1: Removing RoleBinding '${BINDING_NAME}'"
echo "  CMD: oc delete rolebinding ${BINDING_NAME} -n ${NAMESPACE}"
oc delete rolebinding "${BINDING_NAME}" -n "${NAMESPACE}" 2>/dev/null || true
echo "  Restarting rest-service to clear kube-rbac-proxy auth cache..."
oc rollout restart deployment/rest-service -n "${NAMESPACE}"
oc rollout status deployment/rest-service -n "${NAMESPACE}" --timeout=60s

if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token from Test 4"
  echo ""
else
  echo "  CMD: curl $CURL_OPTS -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' ${BASE_URL}${API_PATH}"
  output=$(curl $CURL_OPTS --connect-timeout 5 --max-time 10 \
    -H "Authorization: Bearer $user_token" \
    -H "x-tenant-id: $NAMESPACE" \
    -w '\nHTTP_STATUS:%{http_code}' \
    "${BASE_URL}${API_PATH}" 2>&1) || true
  status=$(echo "$output" | extract_http_status)
  body=$(echo "$output" | sed '/HTTP_STATUS:/d')
  check_result "Valid token, no RoleBinding" \
    "403" \
    "$status (body: $body)" \
    "403"
fi

echo "  Step 2: Restoring RoleBinding '${BINDING_NAME}'"
echo "  CMD: oc apply RoleBinding ${BINDING_NAME} -> ClusterRole/${ROLE} for ${SA_NAME}"
oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: ${BINDING_NAME}
  namespace: $NAMESPACE
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: $ROLE
subjects:
- kind: ServiceAccount
  name: $SA_NAME
  namespace: $NAMESPACE
EOF
echo "  RoleBinding restored"

echo "---"
if [ "$failed_count" -eq 0 ]; then
  echo "ALL PASSED: REST auth tests ($MODE) for $NAMESPACE"
else
  echo "FAILED: $failed_count test(s) failed ($MODE) for $NAMESPACE"
  exit 1
fi
