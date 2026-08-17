#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-infra-example}"
GATEWAY_NS="${2:-openshift-ingress}"

echo ""
echo ""
echo "================================== CREATE ROUTES ============================="
oc apply -f - <<EOF
apiVersion: gateway.networking.k8s.io/v1
kind: HTTPRoute
metadata:
  name: dh-route
  namespace: $NAMESPACE
spec:
  parentRefs:
  - name: dch-gateway
    namespace: $GATEWAY_NS
  rules:
  - matches:
    - path:
        type: PathPrefix
        value: /api/v1/data
    backendRefs:
    - group: ""
      kind: Service
      name: dch-rest-service
      port: 8443
  - matches:
    - path:
        type: PathPrefix
        value: /arrow.flight.protocol.FlightService
    backendRefs:
    - group: ""
      kind: Service
      name: dch-flight-service
      port: 50051
EOF

echo ""
echo "=== Cleaning up old GRPCRoute (if any) ==="
oc delete grpcroute dh-grpc-route -n "$NAMESPACE" 2>/dev/null || true

echo ""
echo "=== Verifying ==="
if oc get httproute dh-route -n "$NAMESPACE" &>/dev/null; then
  echo "  HTTPRoute 'dh-route' exists"
else
  echo "  WARNING: HTTPRoute 'dh-route' not found"
fi

echo ""
echo "=== Done ==="
echo "  Namespace: $NAMESPACE"
echo "  Routes:"
echo "    /api/v1/*                                  -> rest-service:8443"
echo "    /arrow.flight.protocol.FlightService/*     -> flight-service:50051"
