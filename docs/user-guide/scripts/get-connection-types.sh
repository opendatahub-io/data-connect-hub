#!/bin/bash
set -euo pipefail

INFRA_NAMESPACE="${1:-dch-infra-example}"
TENANT_NAMESPACE="${1:-dch-infra-example}"

GATEWAY_NS="${2:-${DCH_GATEWAY_NS:-openshift-ingress}}"

GW_HOST="dch-gateway-data-science-gateway-class.${GATEWAY_NS}.svc"
POD_NAME="dch-test-runner"
API_PATH="/api/v1/data/connection-types"
BASE_URL="https://${GW_HOST}"
SA_NAME="${SA_NAME:-dch-test-user}"

echo "  Creating test runner pod..."
oc get pod "$POD_NAME" -n "$INFRA_NAMESPACE" &>/dev/null || \
oc run "$POD_NAME" -n "$INFRA_NAMESPACE" \
  --image=registry.access.redhat.com/ubi9/ubi:latest \
  --restart=Never \
  --command -- sleep infinity

echo "  Waiting for test runner pod..."
oc wait --for=condition=Ready pod/"$POD_NAME" -n "$INFRA_NAMESPACE" --timeout=60s

. ./get-token.sh

echo "  CMD: curl -sk -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $TENANT_NAMESPACE' ${BASE_URL}${API_PATH}"

oc exec "$POD_NAME" -n "$INFRA_NAMESPACE" -- curl -sk \
    -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" \
    "${BASE_URL}${API_PATH}" | jq . 
