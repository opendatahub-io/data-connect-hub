#!/bin/bash
. ./common-vars.sh

oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: dch-flight-secret-reader
  namespace: $TENANT_NAMESPACE
rules:
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get"]
    resourceNames:
      - dch-database-config
      - $DCH_S3_SECRET_NAME
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: dch-flight-secret-reader
  namespace: $TENANT_NAMESPACE
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: dch-flight-secret-reader
subjects:
  - kind: ServiceAccount
    name: dch-flight-service-sa
    namespace: $INFRA_NAMESPACE
EOF
