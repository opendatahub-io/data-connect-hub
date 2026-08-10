#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
GATEWAY_NS="${2:-${DCH_GATEWAY_NS:-openshift-ingress}}"
GW_SVC="dch-gateway-data-science-gateway-class"
LOCAL_PORT=8443
API_PATH="/api/v1/data/connections"

echo ""
echo ""
echo "================================== VERIFY REST AUTH ============================="
echo "--- Testing rest-service via gateway for $NAMESPACE ---"

extract_http_status() {
  grep 'HTTP_STATUS:' | sed 's/.*HTTP_STATUS://' | grep -o '^[0-9]*'
}

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

cleanup() {
  kill $pf_pid 2>/dev/null || true
  wait $pf_pid 2>/dev/null || true
}
trap cleanup EXIT

fail() {
  echo "  FAILED: $1"
  exit 1
}

user_token=$(oc whoami -t 2>/dev/null) || true
if [ -z "$user_token" ]; then
  echo "  SKIPPED: could not obtain user token (not logged in?)"
  exit 0
fi

echo ""
echo "Test 1: Unauthenticated request (expect 401)..."
echo "  CMD: curl -sk https://localhost:$LOCAL_PORT$API_PATH"
output=$(curl -sk --connect-timeout 5 --max-time 10 \
  -w '\nHTTP_STATUS:%{http_code}' \
  "https://localhost:$LOCAL_PORT$API_PATH" 2>&1)
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
echo "  RESPONSE STATUS: $status"
echo "  RESPONSE BODY: $body"
[ "$status" = "401" ] && echo "  PASSED: unauthenticated request rejected (401)" \
  || fail "expected 401, got $status"

echo ""
echo "Test 2: Non-matching path (expect 404)..."
echo "  CMD: curl -sk -H 'Authorization: Bearer <token>' https://localhost:$LOCAL_PORT/api/v2/data/connections"
output=$(curl -sk --connect-timeout 5 --max-time 10 \
  -H "Authorization: Bearer $user_token" \
  -w '\nHTTP_STATUS:%{http_code}' \
  "https://localhost:$LOCAL_PORT/api/v2/data/connections" 2>&1)
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
echo "  RESPONSE STATUS: $status"
echo "  RESPONSE BODY: $body"
[ "$status" = "404" ] && echo "  PASSED: non-matching path — no route matched (404)" \
  || fail "expected 404, got $status"

echo ""
echo "Test 3: Bad token (expect 401)..."
echo "  CMD: curl -sk -H 'Authorization: Bearer bad-token' https://localhost:$LOCAL_PORT$API_PATH"
output=$(curl -sk --connect-timeout 5 --max-time 10 \
  -H "Authorization: Bearer bad-token" \
  -w '\nHTTP_STATUS:%{http_code}' \
  "https://localhost:$LOCAL_PORT$API_PATH" 2>&1)
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
echo "  RESPONSE STATUS: $status"
echo "  RESPONSE BODY: $body"
[ "$status" = "401" ] && echo "  PASSED: bad token correctly rejected (401)" \
  || fail "expected 401, got $status"

echo ""
echo "Test 4: Authenticated request (expect 200 or 501)..."

echo "  User: $(oc whoami)"
echo "  CMD: curl -sk -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' https://localhost:$LOCAL_PORT$API_PATH"
output=$(curl -sk --connect-timeout 5 --max-time 10 \
  -H "Authorization: Bearer $user_token" \
  -H "x-tenant-id: $NAMESPACE" \
  -w '\nHTTP_STATUS:%{http_code}' \
  "https://localhost:$LOCAL_PORT$API_PATH" 2>&1)
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
echo "  RESPONSE STATUS: $status"
echo "  RESPONSE BODY: $body"
[ "$status" = "200" ] || [ "$status" = "501" ] && echo "  PASSED: authenticated request reached rest-service ($status)" \
  || fail "expected 200 or 501, got $status"

echo ""
echo "ALL PASSED: gateway REST auth tests for $NAMESPACE"
