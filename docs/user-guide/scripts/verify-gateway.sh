#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-openshift-ingress}"

echo ""
echo ""
echo "================================== VERIFY GATEWAY ============================="
echo "  Waiting for gateway to be accepted..."
ACCEPTED=""
for i in $(seq 1 30); do
  ACCEPTED=$(oc get gateway dch-gateway -n "$NAMESPACE" -o jsonpath='{.status.conditions[?(@.type=="Accepted")].status}' 2>/dev/null)
  if [ "$ACCEPTED" = "True" ]; then
    echo "  Gateway accepted after ${i}s"
    break
  fi
  sleep 1
done

if [ "$ACCEPTED" != "True" ]; then
  echo "  WARNING: Gateway not yet accepted (status=$ACCEPTED)"
fi

echo ""
echo "=== Done ==="
echo "  Gateway: dch-gateway"
echo "  Namespace: $NAMESPACE"
echo "  Service: dch-gateway-data-science-gateway-class"
