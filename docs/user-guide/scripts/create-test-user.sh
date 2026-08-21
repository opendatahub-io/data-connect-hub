#!/bin/bash
set -euo pipefail

TENANT_NAMESPACE="${1:-dch-example}"
SA_NAME="${2:-dch-test-user}"

create_sa() {
  local sa_name="$1"
  local ns="$2"

  oc create serviceaccount "$sa_name" -n "$ns" 2>/dev/null || true
  oc apply -f - <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: ${sa_name}-token
  namespace: $ns
  annotations:
    kubernetes.io/service-account.name: $sa_name
type: kubernetes.io/service-account-token
EOF
}

create_sa "$SA_NAME" "$TENANT_NAMESPACE"
