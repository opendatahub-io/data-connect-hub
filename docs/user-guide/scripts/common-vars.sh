#!/bin/bash

export INFRA_NAMESPACE="dch-infra-example"
export TENANT_NAMESPACE="dch-example"
export TENANT_ID="dch-example"
export REST_LOCAL_PORT=18080
export REST_API_BASE="http://localhost:${REST_LOCAL_PORT}/api/v1alpha1/data"

export FLIGHT_LOCAL_PORT=15051
export DCH_S3_SECRET_NAME="s3-test-creds"

export SA_NAME="${SA_NAME:-dch-test-user}"
export GW_HOST=data-science-gateway-data-science-gateway-class.openshift-ingress.svc.cluster.local
export GW_URL="https://${GW_HOST}"
