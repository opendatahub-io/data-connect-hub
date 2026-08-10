#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"

echo ""
echo ""
echo "================================== CREATE ROLES ============================="
echo "--- Creating Roles in $NAMESPACE ---"
oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: dch-ingest
  namespace: $NAMESPACE
rules:
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-connection", "data-store"]
  verbs: ["get"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: dch-admin
  namespace: $NAMESPACE
rules:
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-connection"]
  verbs: ["get", "create", "patch", "delete"]
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-connection-types"]
  verbs: ["get", "create", "patch", "delete"]
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-store"]
  verbs: ["get"]
EOF
echo "PASSED: Roles created"
