#!/bin/bash
set -euo pipefail

. ./common-vars.sh

POD_NAME="dch-test-runner"
API_PATH="/api/v1alpha1/data/connection-types"

echo "  Creating test runner pod..."
oc get pod "$POD_NAME" -n "$INFRA_NAMESPACE" &>/dev/null || \
oc run "$POD_NAME" -n "$INFRA_NAMESPACE" \
  --image=registry.access.redhat.com/ubi9/ubi:latest \
  --restart=Never \
  --command -- sleep infinity

echo "  Waiting for test runner pod..."
oc wait --for=condition=Ready pod/"$POD_NAME" -n "$INFRA_NAMESPACE" --timeout=60s

. ./get-token.sh

CT_DATA='{
"name":"test-postgres-1",
"provider":"postgres",
"credentials_fields":[
  {"name":"URI",
   "label":"URL",
   "type":"string",
   "required":true
  }],
"description":"test connection type"
 }'


oc exec "$POD_NAME" -n "$INFRA_NAMESPACE" -- curl -kX POST -H "Content-Type: application/json" \
    -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" -d "$CT_DATA" \
    "${GW_URL}${API_PATH}" 
