#!/bin/bash
set -euo pipefail

echo ""
echo ""
echo "================================== INSTALL POSTGRES OPERATOR ============================="

echo "  Subscribing to cloudnative-pg operator..."
oc apply -f - <<'EOF'
apiVersion: operators.coreos.com/v1alpha1
kind: Subscription
metadata:
  name: cloudnative-pg
  namespace: openshift-operators
spec:
  channel: stable-v1
  name: cloudnative-pg
  source: certified-operators
  sourceNamespace: openshift-marketplace
  installPlanApproval: Automatic
EOF

echo "  Waiting for operator to be available (up to 150s)..."
echo "  Run: oc get csv -n openshift-operators -l operators.coreos.com/cloudnative-pg.openshift-operators="
for i in $(seq 1 30); do
  csv_phase=$(oc get csv -n openshift-operators -l operators.coreos.com/cloudnative-pg.openshift-operators="" -o jsonpath='{.items[0].status.phase}' 2>/dev/null) || true
  if [ "$csv_phase" = "Succeeded" ]; then
    echo "  Postgres operator installed successfully"
    exit 0
  fi
  if [ "$csv_phase" = "Failed" ]; then
    echo "  FAILED: CSV phase is 'Failed'"
    echo "  Run: oc get csv -n openshift-operators"
    exit 1
  fi
  echo "  Waiting... ($((i * 5))s) phase=${csv_phase:-(not found)}"
  sleep 5
done

echo "  FAILED: operator not ready after 150s"
echo "  The install plan may require manual approval."
echo "  Run: oc get installplan -n openshift-operators | grep cloudnative-pg"
echo "  To approve: oc patch installplan <plan-name> -n openshift-operators --type merge -p '{\"spec\":{\"approved\":true}}'"
exit 1
