#!/bin/bash

export INFRA_NAMESPACE="dch-infra-example"
export TENANT_NAMESPACE="dch-example"
export TENANT_ID="dch-example"
export REST_LOCAL_PORT=18080
export REST_API_BASE="http://localhost:${REST_LOCAL_PORT}/api/v1alpha1/data"

export FLIGHT_LOCAL_PORT=15051
export DCH_S3_SECRET_NAME="s3-test-creds"

export SA_NAME="${SA_NAME:-dch-test-user}"
export GW_HOST=`oc get route data-science-gateway -n openshift-ingress -o jsonpath='{.spec.host}'`
export GW_URL="https://${GW_HOST}"
