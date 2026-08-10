#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
SA_NAME="${2:-dch-test-user}"

echo ""
echo ""
echo "================================== CREATE TEST USER ============================="
echo "=== Creating ServiceAccount '$SA_NAME' in $NAMESPACE ==="
oc create serviceaccount "$SA_NAME" -n "$NAMESPACE" 2>/dev/null || true

echo "=== Creating token Secret ==="
oc apply -f - <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: ${SA_NAME}-token
  namespace: $NAMESPACE
  annotations:
    kubernetes.io/service-account.name: $SA_NAME
type: kubernetes.io/service-account-token
EOF

echo "  Waiting for token to be populated..."
sleep 3

echo SA_NAME=$SA_NAME
echo TOKEN=$(oc get secret "${SA_NAME}-token" -n "$NAMESPACE" -o jsonpath='{.data.token}' | base64 -d)
TOKEN=$(oc get secret "${SA_NAME}-token" -n "$NAMESPACE" -o jsonpath='{.data.token}' | base64 -d)
if [ -z "$TOKEN" ]; then
  echo "FAILED: could not get token for $SA_NAME"
  exit 1
fi

echo "PASSED: ServiceAccount '$SA_NAME' created"
echo "  ServiceAccount: system:serviceaccount:$NAMESPACE:$SA_NAME"
echo "  Token: $TOKEN"
