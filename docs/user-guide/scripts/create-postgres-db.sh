#!/bin/bash
set -euo pipefail

NS="${1:-dch-example}"

echo ""
echo ""
echo "================================== CREATE POSTGRES DB ============================="

echo "  Creating CloudNativePG cluster in namespace '$NS'..."
oc apply -n "$NS" -f - <<'EOF'
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: dch-postgres
spec:
  instances: 1
  storage:
    size: 5Gi
  bootstrap:
    initdb:
      database: dataconnecthub
      owner: dch
EOF

echo "  Waiting for postgres cluster to be ready (up to 180s)..."
echo "  Run: oc get cluster dch-postgres -n $NS"
for i in $(seq 1 36); do
  phase=$(oc get cluster dch-postgres -n "$NS" -o jsonpath='{.status.phase}' 2>/dev/null) || true
  if [ "$phase" = "Cluster in healthy state" ]; then
    echo "  Postgres cluster is ready"
    exit 0
  fi
  echo "  Waiting... ($((i * 5))s) phase=${phase:-(not found)}"
  sleep 5
done

echo "  FAILED: postgres cluster not ready after 180s"
echo "  Run: oc describe cluster dch-postgres -n $NS"
exit 1
