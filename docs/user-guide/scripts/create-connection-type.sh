#!/bin/bash
set -euo pipefail

. ./common-vars.sh

POD_NAME="dch-test-runner"
API_PATH="/api/v1alpha1/data/connection-types"

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


curl -kX POST -H "Content-Type: application/json" -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" -d "$CT_DATA"  "${GW_URL}${API_PATH}" 
