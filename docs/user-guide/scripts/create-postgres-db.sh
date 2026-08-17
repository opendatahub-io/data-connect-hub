#!/bin/bash

NS="${1:-dch-infra-example}"

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
