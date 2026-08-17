#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-infra-example}"
SCRIPT_DIR="$(cd "$(dirname "$0")/../../.." && pwd)"

echo "=== Applying rest-service and flight ==="
oc apply -k "$SCRIPT_DIR/config/base/" -n "$NAMESPACE"

#echo "=== Applying flight-service ==="
#oc apply -k "$SCRIPT_DIR/config/base/flight-service/" -n "$NAMESPACE"
