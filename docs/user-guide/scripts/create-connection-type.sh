#!/bin/bash

. ./common-vars.sh
. ./common-port-forward-rest.sh

CT_DATA='{
"name":"test-postgres-1",
"provider":"postgres",
"description":"test connection type",
"credentials_fields":[
  {"name":"url",
   "label":"URL",
   "type":"string",
   "required":true
  }]
 }'

echo "  CMD: curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: $TENANT_NAMESPACE' -d \"$CT_DATA\" ${REST_API_BASE}/connection-types"
curl -X POST -H "Content-Type: application/json" -H "x-tenant-id: $TENANT_NAMESPACE" -d "$CT_DATA" "${REST_API_BASE}/connection-types" | jq .

cleanup
