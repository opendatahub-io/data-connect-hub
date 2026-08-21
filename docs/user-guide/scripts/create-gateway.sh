#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-openshift-ingress}"

echo ""
echo ""
echo "================================== Pre-req: Create GATEWAY for tenants ============================="
echo "  Creating dch-gateway in namespace '$NAMESPACE'..."

oc apply -f - <<EOF
apiVersion: v1
kind: ConfigMap
metadata:
  name: dch-gateway-config
  namespace: $NAMESPACE
data:
  service: |
    metadata:
      annotations:
        service.beta.openshift.io/serving-cert-secret-name: "dch-gateway-tls"
    spec:
      type: ClusterIP
  deployment: |
    spec:
      template:
        spec:
          containers:
            - name: istio-proxy
              resources:
                limits:
                  cpu: "2"
                  memory: 2Gi
                requests:
                  cpu: 500m
                  memory: 512Mi
---
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: dch-gateway
  namespace: $NAMESPACE
spec:
  gatewayClassName: data-science-gateway-class
  infrastructure:
    parametersRef:
      group: ""
      kind: ConfigMap
      name: dch-gateway-config
  listeners:
  - allowedRoutes:
      namespaces:
        from: All
      kinds:
      - group: gateway.networking.k8s.io
        kind: HTTPRoute
    name: https
    port: 443
    protocol: HTTPS
    tls:
      certificateRefs:
      - group: ""
        kind: Secret
        name: dch-gateway-tls
      mode: Terminate
EOF
