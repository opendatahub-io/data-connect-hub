#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
ROLE="${2:-dch-read}"
SA_NAME="${3:-dch-test-user}"

echo "  Binding '$SA_NAME' to role '$ROLE' in namespace '$NAMESPACE'..."
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
