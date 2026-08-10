#!/bin/bash
set -euo pipefail

OP_NAMESPACE="${1:-dch-example}"
NAMESPACE="${2:-dch-example}"

echo "============================ Creating DCH operator in $OP_NAMESPACE ==="

if ! helm install dc-controller dc-controller/chart/ \
    --namespace "$OP_NAMESPACE" --create-namespace \
    --set controllerManager.image.pullPolicy=Always \
    --set controllerManager.image.tag=latest \
    --set operandNamespace="$NAMESPACE"; then
    echo "ERROR: Helm install failed" >&2
    exit 1
fi
