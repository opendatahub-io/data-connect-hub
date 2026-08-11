#!/bin/bash
set -euo pipefail

MODE="${1:-gateway}"
NAMESPACE="${2:-dch-example}"
GATEWAY_NS="${3:-${DCH_GATEWAY_NS:-openshift-ingress}}"

if [ "$MODE" != "gateway" ] && [ "$MODE" != "pod" ]; then
  echo "Usage: $0 <gateway|pod> [namespace] [gateway-namespace]"
  echo "  gateway  — test via gateway (TLS, port-forward to gateway svc)"
  echo "  pod      — test directly against flight-service pod (plaintext)"
  exit 1
fi

echo ""
echo ""
echo "================================== VERIFY FLIGHT AUTH ============================="

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

if [ "$MODE" = "gateway" ]; then
  GW_SVC="dch-gateway-data-science-gateway-class"
  LOCAL_PORT=9443
  echo "--- Testing flight-service via gateway gRPC for $NAMESPACE ---"

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

  grpc_opts="-insecure -import-path /tmp -proto Flight.proto"
else
  LOCAL_PORT=50051
  echo "--- Testing flight-service directly via pod for $NAMESPACE ---"

  flight_pod=$(oc get po -n "$NAMESPACE" -l app.kubernetes.io/name=flight-service -o jsonpath='{.items[0].metadata.name}')

  lsof -ti :$LOCAL_PORT 2>/dev/null | xargs kill 2>/dev/null || true
  echo "  Waiting for port $LOCAL_PORT to be free..."
  for i in $(seq 1 10); do
    lsof -ti :$LOCAL_PORT &>/dev/null || break
    sleep 1
  done

  echo "  Port-forwarding $flight_pod:50051 -> localhost:$LOCAL_PORT..."
  oc port-forward "$flight_pod" -n "$NAMESPACE" "$LOCAL_PORT:50051" &
  pf_pid=$!
  sleep 2

  grpc_opts="-plaintext -import-path /tmp -proto Flight.proto"
fi

grpc_method="arrow.flight.protocol.FlightService/ListFlights"

proto_file="/tmp/Flight.proto"
if [ ! -f "$proto_file" ]; then
  echo "  Downloading Flight.proto..."
  curl -sL -o "$proto_file" "https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto"
fi

SA_NAME="${SA_NAME:-dch-test-user}"
SA_NOAUTH="${SA_NOAUTH:-dch-test-noauth}"

echo ""
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

echo "Test 1: Unauthenticated gRPC request"
echo "  CMD: grpcurl $grpc_opts -d '{}' localhost:$LOCAL_PORT $grpc_method"
unauth_output=$(grpcurl $grpc_opts -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
check_result "No token" \
  "missing bearer token" \
  "$unauth_output" \
  "missing bearer token"

echo "Test 2: Bad token gRPC request"
echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer bad-token' -H 'x-tenant-id: $NAMESPACE' -d '{}' localhost:$LOCAL_PORT $grpc_method"
bad_token_output=$(grpcurl $grpc_opts -H "Authorization: Bearer bad-token" -H "x-tenant-id: $NAMESPACE" -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
check_result "Bad token" \
  "invalid bearer token" \
  "$bad_token_output" \
  "invalid bearer token"

echo "Test 3: Valid token but missing x-tenant-id (expect missing x-tenant-id)"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -d '{}' localhost:$LOCAL_PORT $grpc_method"
  missing_tenant_output=$(grpcurl $grpc_opts -H "Authorization: Bearer $user_token" -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
  check_result "Valid token, no x-tenant-id" \
    "missing x-tenant-id" \
    "$missing_tenant_output" \
    "missing x-tenant-id"
fi

echo "Test 4: Valid token but wrong x-tenant-id (expect PermissionDenied)"
WRONG_TENANT="default"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $WRONG_TENANT' -d '{}' localhost:$LOCAL_PORT $grpc_method"
  wrong_tenant_output=$(grpcurl $grpc_opts -H "Authorization: Bearer $user_token" -H "x-tenant-id: $WRONG_TENANT" -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
  check_result "Valid token, wrong x-tenant-id (default)" \
    "PermissionDenied" \
    "$wrong_tenant_output" \
    "PermissionDenied"
fi

echo "Test 5: Valid token, no RoleBinding (SA: $SA_NOAUTH, expect PermissionDenied)"
if [ -z "$noauth_token" ]; then
  echo "  SKIPPED  no token for $SA_NOAUTH"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' -d '{}' localhost:$LOCAL_PORT $grpc_method"
  noauth_output=$(grpcurl $grpc_opts -H "Authorization: Bearer $noauth_token" -H "x-tenant-id: $NAMESPACE" -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
  check_result "Valid token, no RoleBinding ($SA_NOAUTH)" \
    "PermissionDenied" \
    "$noauth_output" \
    "PermissionDenied"
fi

echo "Test 6: Authenticated ListFlights (SA: $SA_NAME)"
if [ -z "$user_token" ]; then
  echo "  SKIPPED  no token"
  echo ""
else
  echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' -d '{}' localhost:$LOCAL_PORT $grpc_method"
  auth_output=$(grpcurl $grpc_opts -H "Authorization: Bearer $user_token" -H "x-tenant-id: $NAMESPACE" -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
  check_result "Valid token (SA: $SA_NAME)" \
    "Unimplemented or successful response" \
    "$auth_output" \
    "Unimplemented\|ListFlights"
fi

echo "---"
if [ "$failed_count" -eq 0 ]; then
  echo "ALL PASSED: flight auth tests ($MODE) for $NAMESPACE"
else
  echo "FAILED: $failed_count test(s) failed ($MODE) for $NAMESPACE"
  exit 1
fi
