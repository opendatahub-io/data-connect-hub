#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
GATEWAY_NS="${2:-openshift-ingress}"

echo ""
echo ""
echo "================================== CREATE ROUTES ============================="
echo "=== Checking prerequisites ==="
if ! oc get gateway dch-gateway -n "$GATEWAY_NS" &>/dev/null; then
  echo "ERROR: Gateway 'dch-gateway' does not exist in namespace '$GATEWAY_NS'."
  echo "  Run './docs/user-guide/scripts/create-gateway.sh' first."
  exit 1
fi

if ! oc get service rest-service -n "$NAMESPACE" &>/dev/null; then
  echo "ERROR: Service 'rest-service' does not exist in namespace '$NAMESPACE'."
  exit 1
fi

if ! oc get service flight-service -n "$NAMESPACE" &>/dev/null; then
  echo "ERROR: Service 'flight-service' does not exist in namespace '$NAMESPACE'."
  exit 1
fi

echo "  All prerequisites met."

echo ""
echo "=== Creating HTTPRoute in $NAMESPACE ==="
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
        value: /api/v1
    backendRefs:
    - group: ""
      kind: Service
      name: rest-service
      port: 8443
  - matches:
    - path:
        type: PathPrefix
        value: /arrow.flight.protocol.FlightService
    backendRefs:
    - group: ""
      kind: Service
      name: flight-service
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
