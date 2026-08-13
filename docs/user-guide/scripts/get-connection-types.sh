#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
GATEWAY_NS="${2:-${DCH_GATEWAY_NS:-openshift-ingress}}"

echo ""
echo ""
echo "================================== GET CONNECTION TYPES ============================="

GW_HOST="dch-gateway-data-science-gateway-class.${GATEWAY_NS}.svc"
POD_NAME="dch-test-runner"
API_PATH="/api/v1/data/connection-types"
BASE_URL="https://${GW_HOST}"
SA_NAME="${SA_NAME:-dch-test-user}"

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
  oc exec "$POD_NAME" -n "$NAMESPACE" -- curl -sk \
    --connect-timeout 5 --max-time 10 "$@" 2>&1
}

echo "--- Setup ---"

echo "  Creating test runner pod..."
oc get pod "$POD_NAME" -n "$NAMESPACE" &>/dev/null || \
oc run "$POD_NAME" -n "$NAMESPACE" \
  --image=registry.access.redhat.com/ubi9/ubi:latest \
  --restart=Never \
  --command -- sleep infinity

echo "  Waiting for test runner pod..."
oc wait --for=condition=Ready pod/"$POD_NAME" -n "$NAMESPACE" --timeout=60s

echo "--- Obtaining token ---"
SA_ISSUER=$(oc get authentication cluster -o jsonpath='{.spec.serviceAccountIssuer}' 2>/dev/null) || true
if [ -z "$SA_ISSUER" ]; then
  SA_ISSUER="https://kubernetes.default.svc"
fi
echo "  Using audience: $SA_ISSUER"

. ./get-token.sh

if [ -z "$user_token" ]; then
  echo "  WARNING: could not obtain token for $SA_NAME (run create-test-users.sh first)"
fi
echo ""

echo "--- Testing rest-service via gateway ($GW_HOST) for $NAMESPACE ---"
echo ""

echo "Test 1: GET /connection-types (expect 200)"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: curl -sk -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' ${BASE_URL}${API_PATH}"
  output=$(run_curl -H "Authorization: Bearer $user_token" -H "x-tenant-id: $NAMESPACE" \
    -w '\nHTTP_STATUS:%{http_code}' "${BASE_URL}${API_PATH}") || true
  status=$(echo "$output" | extract_http_status)
  body=$(echo "$output" | sed '/HTTP_STATUS:/d')
  echo "  body: $(echo "$body" | jq . 2>/dev/null || echo "$body")"
  check_result "GET /connection-types" "200" "$status"
fi

echo "---"
if [ "$failed_count" -eq 0 ]; then
  echo "ALL PASSED: GET connection-types for $NAMESPACE"
else
  echo "FAILED: $failed_count test(s) failed for $NAMESPACE"
  exit 1
fi
