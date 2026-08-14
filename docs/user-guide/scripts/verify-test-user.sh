
#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
ROLE="${2:-dch-read}"
SA_NAME="${3:-dch-test-user}"
SA_NOAUTH="${3:-dch-test-noauth}"

SA_FULL="system:serviceaccount:$NAMESPACE:$SA_NAME"

verify_sa() {
  local sa_name="$1"
  local ns="$2"

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

verify_sa "$SA_NAME" "$NAMESPACE"
verify_sa "$SA_NOAUTH" "$NAMESPACE"

echo "=== Verifying ==="
if oc get rolebinding "${SA_NAME}-${ROLE}" -n "$NAMESPACE" &>/dev/null; then
  echo "  RoleBinding '${SA_NAME}-${ROLE}' exists"
else
  echo "  WARNING: RoleBinding '${SA_NAME}-${ROLE}' not found"
fi

