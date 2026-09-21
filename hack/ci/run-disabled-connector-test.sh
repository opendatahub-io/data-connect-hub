#!/usr/bin/env bash
# Verify CRUD and ingestion behavior for a disabled connector.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

E2E_DISABLED_CONNECTOR="${E2E_DISABLED_CONNECTOR:-}"

if [[ -z "$E2E_DISABLED_CONNECTOR" ]]; then
    echo "=== Skipping disabled connector E2E test: E2E_DISABLED_CONNECTOR is empty ==="
    exit 0
fi

if ! has_connector "$E2E_DISABLED_CONNECTOR"; then
    echo "ERROR: E2E_DISABLED_CONNECTOR must be included in E2E_CONNECTORS" >&2
    exit 1
fi

echo "=== Disabling ${E2E_DISABLED_CONNECTOR} connector ==="
kubectl patch "dataconnectservices.dataconnecthub.opendatahub.io/${DCS_NAME}" \
    -n "$SVC_NAMESPACE" \
    --type=merge \
    -p "{\"spec\":{\"flightService\":{\"connectors\":[{\"name\":\"${E2E_DISABLED_CONNECTOR}\",\"enabled\":false}]}}}"

generation=$(kubectl get "dataconnectservices.dataconnecthub.opendatahub.io/${DCS_NAME}" \
    -n "$SVC_NAMESPACE" \
    -o jsonpath='{.metadata.generation}')

echo "=== Waiting for DataConnectService reconcile ==="
kubectl wait \
    --for="jsonpath={.status.observedGeneration}=${generation}" \
    "dataconnectservices.dataconnecthub.opendatahub.io/${DCS_NAME}" \
    -n "$SVC_NAMESPACE" \
    --timeout=180s

echo "=== DataConnectService CR after disabling URI ==="
kubectl get "dataconnectservices.dataconnecthub.opendatahub.io/${DCS_NAME}" \
    -n "$SVC_NAMESPACE" \
    -o yaml

echo "=== Verifying Flight Service config.toml ==="
config_toml=$(kubectl get "configmap/${FLIGHT_SERVICE_NAME}-config" \
    -n "$SVC_NAMESPACE" \
    -o jsonpath='{.data.config\.toml}')
printf '%s\n' "$config_toml"
E2E_DISABLED_CONNECTOR="$E2E_DISABLED_CONNECTOR" CONFIG_TOML="$config_toml" python3 - <<'PY'
import os
import tomllib

config = tomllib.loads(os.environ["CONFIG_TOML"])
connector_name = os.environ["E2E_DISABLED_CONNECTOR"]
connector = config.get("connectors", {}).get(connector_name)
if connector is None:
    raise SystemExit(f"[connectors.{connector_name}] is missing from config.toml")
if connector.get("enabled") is not False:
    raise SystemExit(f"[connectors.{connector_name}].enabled is not false in config.toml")

print(f"Verified [connectors.{connector_name}].enabled = false")
PY

if ! kubectl wait \
    --for=jsonpath='{.status.phase}'=Ready \
    "dataconnectservices.dataconnecthub.opendatahub.io/${DCS_NAME}" \
    -n "$SVC_NAMESPACE" \
    --timeout=180s; then
    kubectl get "dataconnectservices.dataconnecthub.opendatahub.io/${DCS_NAME}" \
        -n "$SVC_NAMESPACE" \
        -o yaml || true
    echo "ERROR: DataConnectService did not become Ready after disabling URI" >&2
    exit 1
fi

echo "=== Waiting for Flight Service rollout ==="
kubectl rollout status "deployment/${FLIGHT_SERVICE_NAME}" \
    -n "$SVC_NAMESPACE" \
    --timeout=180s

echo "=== Running disabled connector E2E test ==="
E2E_DISABLED_CONNECTOR="$E2E_DISABLED_CONNECTOR" \
    "$REPO_ROOT/e2e/.venv/bin/pytest" \
    "$REPO_ROOT/e2e/ci_tests/test_disabled_connector.py" \
    -v \
    -s
