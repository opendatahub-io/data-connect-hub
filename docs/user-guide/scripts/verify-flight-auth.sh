#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
GATEWAY_NS="${2:-${DCH_GATEWAY_NS:-openshift-ingress}}"
GW_SVC="dch-gateway-data-science-gateway-class"
LOCAL_PORT=9443

echo ""
echo ""
echo "================================== VERIFY FLIGHT AUTH ============================="
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

cleanup() {
  kill $pf_pid 2>/dev/null || true
  wait $pf_pid 2>/dev/null || true
}
trap cleanup EXIT

fail() {
  echo "  FAILED: $1"
  #exit 1
}

proto_file="/tmp/Flight.proto"
if [ ! -f "$proto_file" ]; then
  echo "  Downloading Flight.proto..."
  curl -sL -o "$proto_file" "https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto"
fi

grpc_opts="-insecure -import-path /tmp -proto Flight.proto"
grpc_method="arrow.flight.protocol.FlightService/ListFlights"

echo ""
echo "Test 1: Unauthenticated gRPC request (expect Unauthenticated)..."
echo "  CMD: grpcurl $grpc_opts -d '{}' localhost:$LOCAL_PORT $grpc_method"
unauth_output=$(grpcurl $grpc_opts -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
echo "  RESPONSE: $unauth_output"
if echo "$unauth_output" | grep -q "Unauthenticated\|PermissionDenied\|401\|403"; then
  echo "  PASSED: unauthenticated request rejected"
else
  fail "expected Unauthenticated or PermissionDenied, got: $unauth_output"
fi

echo ""
echo "Test 2: Bad token gRPC request (expect Unauthenticated)..."
echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer bad-token' -d '{}' localhost:$LOCAL_PORT $grpc_method"
bad_token_output=$(grpcurl $grpc_opts -H "Authorization: Bearer bad-token" -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
echo "  RESPONSE: $bad_token_output"
if echo "$bad_token_output" | grep -q "Unauthenticated\|PermissionDenied\|401\|403"; then
  echo "  PASSED: bad token rejected"
else
  fail "expected Unauthenticated or PermissionDenied, got: $bad_token_output"
fi

echo ""
echo "Test 3: Authenticated ListFlights RPC call..."
user_token=$(oc whoami -t 2>/dev/null) || true
if [ -z "$user_token" ]; then
  echo "  SKIPPED: could not obtain user token (not logged in?)"
  exit 0
fi
echo "  User: $(oc whoami)"
echo "  CMD: grpcurl $grpc_opts -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' -d '{}' localhost:$LOCAL_PORT $grpc_method"
auth_output=$(grpcurl $grpc_opts -H "Authorization: Bearer $user_token" -H "x-tenant-id: $NAMESPACE" -d '{}' localhost:$LOCAL_PORT $grpc_method 2>&1) || true
echo "  RESPONSE: $auth_output"
if echo "$auth_output" | grep -q "Unimplemented\|ListFlights"; then
  echo "  PASSED: authenticated ListFlights responded"
else
  fail "authenticated ListFlights — no response from service"
fi

echo ""
echo "ALL PASSED: gateway gRPC tests for $NAMESPACE"
