#!/bin/bash
set -euo pipefail

echo ""
echo ""
echo "================================== CREATE ROLES ============================="
echo "--- Creating ClusterRoles ---"
oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: dch-ingest
rules:
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-connections", "data-store"]
  verbs: ["get"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: dch-admin
rules:
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-connections"]
  verbs: ["get", "create", "patch", "delete"]
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-connection-types"]
  verbs: ["get", "create", "patch", "delete"]
- apiGroups: ["dataconnecthub.opendatahub.io"]
  resources: ["data-store"]
  verbs: ["get"]
EOF
echo "PASSED: ClusterRoles created"
