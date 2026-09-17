#!/usr/bin/env bash
# Verify CRUD and ingestion behavior for a disabled connector.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

if [[ -z "$CI_DISABLED_CONNECTORS" ]]; then
    echo "=== Skipping disabled connector E2E test: CI_DISABLED_CONNECTORS is empty ==="
    exit 0
fi

if ! has_connector "$CI_DISABLED_CONNECTORS"; then
    echo "ERROR: CI_DISABLED_CONNECTORS must be included in CI_ENABLED_CONNECTORS" >&2
    exit 1
fi

echo "=== Disabling ${CI_DISABLED_CONNECTORS} connector ==="
kubectl patch "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" \
    -n "$CI_SVC_NAMESPACE" \
    --type=merge \
    -p "{\"spec\":{\"flightService\":{\"connectors\":[{\"name\":\"${CI_DISABLED_CONNECTORS}\",\"enabled\":false}]}}}"

generation=$(kubectl get "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" \
    -n "$CI_SVC_NAMESPACE" \
    -o jsonpath='{.metadata.generation}')

echo "=== Waiting for DataConnectService reconcile ==="
kubectl wait \
    --for="jsonpath={.status.observedGeneration}=${generation}" \
    "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" \
    -n "$CI_SVC_NAMESPACE" \
    --timeout=180s

echo "=== DataConnectService CR after disabling URI ==="
kubectl get "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" \
    -n "$CI_SVC_NAMESPACE" \
    -o yaml

echo "=== Verifying Flight Service config.toml ==="
config_toml=$(kubectl get "configmap/${CI_FLIGHT_SERVICE_NAME}-config" \
    -n "$CI_SVC_NAMESPACE" \
    -o jsonpath='{.data.config\.toml}')
printf '%s\n' "$config_toml"
disabled_connector="$CI_DISABLED_CONNECTORS" config_toml="$config_toml" python3 - <<'PY'
import os
import tomllib

config = tomllib.loads(os.environ["config_toml"])
connector_name = os.environ["disabled_connector"]
connector = config.get("connectors", {}).get(connector_name)
if connector is None:
    raise SystemExit(f"[connectors.{connector_name}] is missing from config.toml")
if connector.get("enabled") is not False:
    raise SystemExit(f"[connectors.{connector_name}].enabled is not false in config.toml")

print(f"Verified [connectors.{connector_name}].enabled = false")
PY

if ! kubectl wait \
    --for=jsonpath='{.status.phase}'=Ready \
    "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" \
    -n "$CI_SVC_NAMESPACE" \
    --timeout=180s; then
    kubectl get "dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}" \
        -n "$CI_SVC_NAMESPACE" \
        -o yaml || true
    echo "ERROR: DataConnectService did not become Ready after disabling URI" >&2
    exit 1
fi

echo "=== Waiting for Flight Service rollout ==="
kubectl rollout status "deployment/${CI_FLIGHT_SERVICE_NAME}" \
    -n "$CI_SVC_NAMESPACE" \
    --timeout=180s

echo "=== Running disabled connector E2E test ==="
DCH_DISABLED_CONNECTORS="$CI_DISABLED_CONNECTORS" \
    "$CI_REPO_ROOT/e2e/.venv/bin/pytest" \
    "$CI_REPO_ROOT/e2e/ci_tests/test_disabled_connector.py" \
    -v \
    -s
