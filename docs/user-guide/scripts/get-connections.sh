#!/bin/bash
set -euo pipefail

. ./common-vars.sh

POD_NAME="dch-test-runner"
API_PATH="/api/v1alpha1/data/connections"

. ./get-token.sh

echo curl -k -H "Authorization: Bearer <user_token>" -H "x-tenant-id: $TENANT_NAMESPACE" "${GW_URL}${API_PATH}"
curl -k -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" "${GW_URL}${API_PATH}" | jq .
