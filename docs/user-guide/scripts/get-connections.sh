#!/bin/bash
set -euo pipefail

. ./common-vars.sh

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

echo oc exec "$POD_NAME" -n "$INFRA_NAMESPACE" -- curl -k \
    -H "Authorization: Bearer <user_token>" -H "x-tenant-id: $TENANT_NAMESPACE" \
    "${GW_URL}${API_PATH}"
oc exec "$POD_NAME" -n "$INFRA_NAMESPACE" -- curl -k \
    -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" \
    "${GW_URL}${API_PATH}" | jq .
