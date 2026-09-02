#!/bin/bash
set -euo pipefail

INFRA_NAMESPACE="${1:-dch-infra-example}"
TENANT_NAMESPACE="${2:-dch-example}"

echo "  Extracting database URI from secret 'dch-postgres-app' in namespace '$INFRA_NAMESPACE'..."
URI=$(oc get secret dch-postgres-app -n "$INFRA_NAMESPACE" -o jsonpath='{.data.uri}' 2>/dev/null | base64 -d) || true

if [ -z "$URI" ]; then
  echo "  FAILED: could not extract URI from secret 'dch-postgres-app'"
  echo "  Run: oc get secret dch-postgres-app -n $INFRA_NAMESPACE -o yaml"
  exit 1
fi

echo "  Creating secret 'dch-database-config' in namespace '$TENANT_NAMESPACE'..."
oc apply -n "$TENANT_NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: dch-database-config
stringData:
  DATABASE_URL: "$URI"
  URI: "$URI"
  secret-config.toml: |
    [database]
    URI = "$URI"
EOF
