#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
SA_NAME="${2:-dch-test-user}"
SA_NOAUTH="${3:-dch-test-noauth}"

echo ""
echo ""
echo "================================== CREATE TEST USERS ============================="

create_sa() {
  local sa_name="$1"
  local ns="$2"

  echo "=== Creating ServiceAccount '$sa_name' in $ns ==="
  oc create serviceaccount "$sa_name" -n "$ns" 2>/dev/null || true

  echo "=== Creating token Secret ==="
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

  echo "  Waiting for token to be populated..."
  sleep 3

  local token
  token=$(oc get secret "${sa_name}-token" -n "$ns" -o jsonpath='{.data.token}' | base64 -d)
  if [ -z "$token" ]; then
    echo "FAILED: could not get token for $sa_name"
    return 1
  fi

  echo "PASSED: ServiceAccount '$sa_name' created"
  echo "  ServiceAccount: system:serviceaccount:$ns:$sa_name"
  echo ""
}

create_sa "$SA_NAME" "$NAMESPACE"
create_sa "$SA_NOAUTH" "$NAMESPACE"
