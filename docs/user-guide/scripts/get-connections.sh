#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"

echo ""
echo ""
echo "================================== GET CONNECTIONS ============================="

LOCAL_PORT=18080
API_BASE="http://localhost:${LOCAL_PORT}/api/v1/data"
TENANT_ID="$NAMESPACE"
pf_pid=""

cleanup() {
  if [ -n "$pf_pid" ]; then
    kill $pf_pid 2>/dev/null || true
    wait $pf_pid 2>/dev/null || true
  fi
}

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
    echo ""
    cleanup
    exit 1
  fi
  echo ""
}

extract_http_status() {
  grep 'HTTP_STATUS:' | sed 's/.*HTTP_STATUS://' | grep -o '^[0-9]*'
}

run_curl() {
  curl -s -H "x-tenant-id: $TENANT_ID" --connect-timeout 5 --max-time 10 "$@" 2>&1
}

echo "--- Setup ---"

echo "  Finding rest-service pod..."
rest_pod=$(oc get po -n "$NAMESPACE" -l app.kubernetes.io/name=rest-service -o jsonpath='{.items[0].metadata.name}' 2>/dev/null) || true
if [ -z "$rest_pod" ]; then
  echo "  FAILED: no rest-service pod found in namespace '$NAMESPACE'"
  exit 1
fi
echo "  Pod: $rest_pod"

echo "  Port-forwarding $rest_pod:8080 -> localhost:$LOCAL_PORT..."
lsof -ti :$LOCAL_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc port-forward "pod/$rest_pod" -n "$NAMESPACE" "$LOCAL_PORT:8080" &>/dev/null &
pf_pid=$!
sleep 2

if ! kill -0 $pf_pid 2>/dev/null; then
  echo "  FAILED: port-forward died"
  exit 1
fi
echo "  Port-forward ready (pid=$pf_pid)"
echo ""

echo "--- Testing rest-service API (localhost:$LOCAL_PORT) for $NAMESPACE ---"
echo ""

echo "Test 6: GET /connections (expect 200)"
echo "  CMD: curl -H 'x-tenant-id: $TENANT_ID' ${API_BASE}/connections"
output=$(run_curl -w '\nHTTP_STATUS:%{http_code}' "${API_BASE}/connections") || true
status=$(echo "$output" | extract_http_status)
body=$(echo "$output" | sed '/HTTP_STATUS:/d')
echo "  body: $(echo "$body" | jq . 2>/dev/null || echo "$body")"
check_result "GET /connections" "200" "$status"
