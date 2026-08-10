
#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
ROLE="${2:-dch-ingest}"
SA_NAME="${3:-dch-test-user}"

SA_FULL="system:serviceaccount:$NAMESPACE:$SA_NAME"

echo "=== Verifying ==="
if oc get rolebinding "${SA_NAME}-${ROLE}" -n "$NAMESPACE" &>/dev/null; then
  echo "  RoleBinding '${SA_NAME}-${ROLE}' exists"
else
  echo "  WARNING: RoleBinding '${SA_NAME}-${ROLE}' not found"
fi
