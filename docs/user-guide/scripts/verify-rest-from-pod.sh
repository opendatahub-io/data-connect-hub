#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"

echo ""
echo ""
echo "================================== VERIFY REST SERVICE ============================="
rest_pod=$(oc get po -n "$NAMESPACE" -l app.kubernetes.io/name=rest-service -o jsonpath='{.items[0].metadata.name}')

lsof -ti :8080 2>/dev/null | xargs kill 2>/dev/null || true
sleep 1
echo "  CMD: oc port-forward $rest_pod -n $NAMESPACE 8080:8080"
oc port-forward "$rest_pod" -n "$NAMESPACE" 8080:8080 &
pf_pid=$!
sleep 2

echo "  CMD: curl -s -H 'x-tenant-id: $NAMESPACE' http://localhost:8080/api/v1/data/connections"
rest_response=$(curl -s -H "x-tenant-id: $NAMESPACE" http://localhost:8080/api/v1/data/connections)
echo "  RESPONSE: $rest_response"

kill $pf_pid 2>/dev/null
wait $pf_pid 2>/dev/null

if [ -z "$rest_response" ] || echo "$rest_response" | grep -q '"code":"header_not_found"'; then
  echo "FAILED: rest-service returned empty or error response"
  exit 1
fi
echo "PASSED: rest-service responded"
