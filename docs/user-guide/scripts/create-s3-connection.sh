#!/bin/bash

. ./common-vars.sh
DCH_S3_TYPE_ID="${1}"

CT_DATA="{\
\"name\":\"s3-conn\",\
\"data_connection_type_id\": \"${DCH_S3_TYPE_ID}\",\
\"format\": \"tabular\",\
\"admin\": {\"secret_ref\": \"s3-test-creds\"},\
\"properties\": {}\
 }"

echo  $CT_DATA | jq .

CMD="curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: $TENANT_ID' -d '$CT_DATA' ${REST_API_BASE}/connections"
echo $CMD 
eval $CMD
