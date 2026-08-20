#!/bin/bash
. ./common-vars.sh
. ./common-port-forward-rest.sh

CT_DATA='{
"name":"test-s3",
"provider":"s3",
"description":"test AWS S3 connection type",
"credentials_fields":[
  {"name": "AWS_S3_BUCKET", "label": "Bucket", "type": "string", "required": true},
  {"name": "AWS_ACCESS_KEY_ID", "label": "Access Key ID", "type": "string", "required": true},
  {"name": "AWS_SECRET_ACCESS_KEY", "label": "Secret Access Key", "type": "string", "required": true}
  ]
 }'

CMD="curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: $TENANT_ID' -d '$CT_DATA' ${REST_API_BASE}/connection-types"
echo $CMD
eval $CMD

cleanup
