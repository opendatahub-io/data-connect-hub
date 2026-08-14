#!/bin/bash
NAMESPACE="${1:-dch-example}"

oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: dch-flight-secret-reader
  namespace: $NAMESPACE
rules:
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get"]
    resourceNames:
      - dch-database-config
      - s3-test-creds
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: dch-flight-secret-reader
  namespace: $NAMESPACE
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: dch-flight-secret-reader
subjects:
  - kind: ServiceAccount
    name: flight-service-sa
    namespace: $NAMESPACE
EOF
