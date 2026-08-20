#!/bin/bash
set -euo pipefail

INFRA_NAMESPACE="${1:-dch-infra-example}"

oc apply -f - <<EOF
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
  namespace: $INFRA_NAMESPACE
spec:
  restService: {}
  flightService: {}
  gateway:
    name: dch-gateway
    namespace: openshift-ingress
EOF
