#!/bin/bash
set -euo pipefail

INFRA_NAMESPACE="${1:-dch-infra-example}"
NAMESPACE="${1:-dch-example}"

oc new-project "$NAMESPACE" 2>/dev/null || oc project "$NAMESPACE"
oc new-project "$INFRA_NAMESPACE" 2>/dev/null || oc project "$INFRA_NAMESPACE"
