#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
oc new-project "$NAMESPACE" 2>/dev/null || oc project "$NAMESPACE"