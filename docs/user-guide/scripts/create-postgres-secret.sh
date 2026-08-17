#!/bin/bash
set -euo pipefail

NS="${1:-dch-infra-example}"

echo "  Extracting database URI from secret 'dch-postgres-app' in namespace '$NS'..."
URI=$(oc get secret dch-postgres-app -n "$NS" -o jsonpath='{.data.uri}' 2>/dev/null | base64 -d) || true

if [ -z "$URI" ]; then
  echo "  FAILED: could not extract URI from secret 'dch-postgres-app'"
  echo "  Run: oc get secret dch-postgres-app -n $NS -o yaml"
  exit 1
fi

echo "  Creating secret 'dch-database-config' in namespace '$NS'..."
oc apply -n "$NS" -f - <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: dch-database-config
stringData:
  DATABASE_URL: "$URI"
  url: "$URI"
  secret-config.toml: |
    [database]
    url = "$URI"
EOF
