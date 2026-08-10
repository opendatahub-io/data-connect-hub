#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"

echo ""
echo ""
echo "================================== CONFIG KUBE-RBAC-PROXY ============================="
echo "--- Creating ClusterRole and ClusterRoleBinding for kube-rbac-proxy in $NAMESPACE ---"
oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: dch-rbac-proxy
rules:
- apiGroups: ["authentication.k8s.io"]
  resources: ["tokenreviews"]
  verbs: ["create"]
- apiGroups: ["authorization.k8s.io"]
  resources: ["subjectaccessreviews"]
  verbs: ["create"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: dch-rbac-proxy
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: dch-rbac-proxy
subjects:
- kind: ServiceAccount
  name: data-connect-hub-sa
  namespace: $NAMESPACE
- kind: ServiceAccount
  name: flight-service-sa
  namespace: $NAMESPACE
EOF
echo "PASSED: kube-rbac-proxy RBAC configured"
