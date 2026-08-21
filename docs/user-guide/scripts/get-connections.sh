#!/bin/bash
set -euo pipefail

. ./common-vars.sh
. ./common-port-forward-rest.sh

echo "  CMD: curl -H 'x-tenant-id: $TENANT_ID' ${REST_API_BASE}/connections"
curl -s -H "x-tenant-id: $TENANT_ID" "${REST_API_BASE}/connections" | jq .

cleanup
