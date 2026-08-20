#!/bin/bash

INFRA_NAMESPACE="dch-infra-example"
TENANT_NAMESPACE="dch-example"
TENANT_ID="dch-example"
REST_LOCAL_PORT=18080
REST_API_BASE="http://localhost:${REST_LOCAL_PORT}/api/v1/data"

FLIGHT_LOCAL_PORT=15051
DCH_S3_SECRET_NAME="s3-test-creds"

SA_NAME="${SA_NAME:-dch-test-user}"
