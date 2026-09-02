#!/bin/bash
set -euo pipefail

. ./common-vars.sh
type_id="${1}"

POD_NAME="dch-test-runner"
API_PATH="/api/v1alpha1/data/connections"

echo "  Creating test runner pod..."
oc get pod "$POD_NAME" -n "$INFRA_NAMESPACE" &>/dev/null || \
oc run "$POD_NAME" -n "$INFRA_NAMESPACE" \
  --image=registry.access.redhat.com/ubi9/ubi:latest \
  --restart=Never \
  --command -- sleep infinity

echo "  Waiting for test runner pod..."
oc wait --for=condition=Ready pod/"$POD_NAME" -n "$INFRA_NAMESPACE" --timeout=60s

. ./get-token.sh

secret_name="dch-database-config"

CT_DATA="{
\"name\":\"test-pg-conn-1\",
\"data_connection_type_id\": \"${type_id}\",
\"format\": \"tabular\",
\"credentials_ref\": {\"secret\": \"${secret_name}\"},
\"properties\": {}
 }"

echo CT_DATA=$CT_DATA

oc exec "$POD_NAME" -n "$INFRA_NAMESPACE" -- curl -k -X POST -H "Content-Type: application/json" \
    -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" -d "$CT_DATA" \
    "${GW_URL}${API_PATH}" 
