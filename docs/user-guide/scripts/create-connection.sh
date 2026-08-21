#!/bin/bash
. ./common-vars.sh
. ./common-port-forward-rest.sh

type_id="${1}"
secret_name="dch-database-config"

CT_DATA="{
\"name\":\"test-pg-conn-1\",
\"data_connection_type_id\": \"${type_id}\",
\"format\": \"tabular\",
\"admin\": {\"secret_ref\": \"${secret_name}\"},
\"properties\": {}
 }"

echo "  CMD: curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: $TENANT_ID' -d \"$CT_DATA\" ${REST_API_BASE}/connections"
curl -X POST -H "Content-Type: application/json" -H "x-tenant-id: $TENANT_ID" -d "$CT_DATA" "${REST_API_BASE}/connections" | jq .

cleanup

