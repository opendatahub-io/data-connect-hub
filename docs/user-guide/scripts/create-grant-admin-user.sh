#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
ROLE="${2:-dch-admin}"
SA_NAME="${3:-dch-test-admin}"

echo "=== Checking prerequisites ==="
if ! oc get clusterrole "$ROLE" &>/dev/null; then
  echo "ERROR: ClusterRole '$ROLE' does not exist."
  echo "  Run './dch.sh -c create_rbac_resources' first to create the DCH roles."
  exit 1
fi

echo "  All prerequisites met."
echo ""
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

TOKEN=$(oc get secret "${SA_NAME}-token" -n "$NAMESPACE" -o jsonpath='{.data.token}' | base64 -d)
if [ -z "$TOKEN" ]; then
  echo "FAILED: could not get token for $SA_NAME"
  exit 1
fi

echo "=== Binding '$SA_NAME' to role '$ROLE' ==="
oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: ${SA_NAME}-${ROLE}
  namespace: $NAMESPACE
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: $ROLE
subjects:
- kind: ServiceAccount
  name: $SA_NAME
  namespace: $NAMESPACE
EOF

SA_FULL="system:serviceaccount:$NAMESPACE:$SA_NAME"

echo "=== Verifying ==="
if oc get rolebinding "${SA_NAME}-${ROLE}" -n "$NAMESPACE" &>/dev/null; then
  echo "  RoleBinding '${SA_NAME}-${ROLE}' exists"
else
  echo "  WARNING: RoleBinding '${SA_NAME}-${ROLE}' not found"
fi

echo ""
echo "=== Done ==="
echo "  ServiceAccount: $SA_FULL"
echo "  Role:           $ROLE"
echo "  Token:"
echo "  $TOKEN"
echo ""
echo "  Example usage:"
echo "  curl -sk -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' https://<host>/api/v1/data/connections"
echo "  curl -sk -X POST -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $NAMESPACE' -H 'Content-Type: application/json' -d '{}' https://<host>/api/v1/data/connections"
