#!/bin/bash

INFRA_NAMESPACE="${1:-dch-infra-example}"
TENANT_NAMESPACE="${2:-dch-example}"
LOCAL_PORT=18080
API_BASE="http://localhost:${LOCAL_PORT}/api/v1/data"
pf_pid=""

echo "  Finding rest-service pod..."
rest_pod=$(oc get po -n "$INFRA_NAMESPACE" -l app.kubernetes.io/name=rest-service -o jsonpath='{.items[0].metadata.name}' 2>/dev/null) || true
if [ -z "$rest_pod" ]; then
  echo "  FAILED: no rest-service pod found in namespace '$INFRA_NAMESPACE'"
  exit 1
fi
echo "  Pod: $rest_pod"

echo "  Port-forwarding $rest_pod:8080 -> localhost:$LOCAL_PORT..."
lsof -ti :$LOCAL_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc port-forward "pod/$rest_pod" -n "$INFRA_NAMESPACE" "$LOCAL_PORT:8080" &>/dev/null &
pf_pid=$!
sleep 2

if ! kill -0 $pf_pid 2>/dev/null; then
  echo "  FAILED: port-forward died"
  exit 1
fi
echo "  Port-forward ready (pid=$pf_pid)"
echo ""

CT_DATA='{
"name":"test-postgres",
"provider":"postgresql",
"description":"test connection type",
"credentials_fields":[
  {"name":"url",
   "label":"URL",
   "type":"string",
   "required":true
  }]
 }'

echo "  CMD: curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: $TENANT_NAMESPACE' -d \"$CT_DATA\" ${API_BASE}/connection-types"
curl -X POST -H "Content-Type: application/json" -H "x-tenant-id: $TENANT_NAMESPACE" -d "$CT_DATA" "${API_BASE}/connection-types" | jq .
