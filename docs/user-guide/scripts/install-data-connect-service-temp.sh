#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
SCRIPT_DIR="$(cd "$(dirname "$0")/../../.." && pwd)"

echo ""
echo ""
echo "================================== INSTALL DATA CONNECT SERVICE ============================="
echo "=== Checking prerequisites ==="
if ! oc get namespace "$NAMESPACE" &>/dev/null; then
  echo "ERROR: Namespace '$NAMESPACE' does not exist."
  exit 1
fi

#if [ ! -f "$SCRIPT_DIR/config/db/postgres/generate-secrets.sh" ]; then
#  echo "ERROR: generate-secrets.sh not found at $SCRIPT_DIR/config/db/postgres/"
#  exit 1
#fi

echo "  All prerequisites met."
echo ""
#echo "=== Generating database secrets ==="
#"$SCRIPT_DIR/config/db/postgres/generate-secrets.sh" || true

#echo "=== Applying database config ==="
#oc apply -k "$SCRIPT_DIR/config/db/postgres" -n "$NAMESPACE"

echo "=== Applying rest-service and flight ==="
oc apply -k "$SCRIPT_DIR/config/base/" -n "$NAMESPACE"

#echo "=== Applying flight-service ==="
#oc apply -k "$SCRIPT_DIR/config/base/flight-service/" -n "$NAMESPACE"
