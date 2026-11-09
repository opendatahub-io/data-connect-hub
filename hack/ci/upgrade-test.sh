#!/usr/bin/env bash
# Stage 4: Upgrade test — automate docs/user-guide/upgrade.md.
#
# Installs the released baseline (odh-3.6), seeds data through the running
# services, upgrades to the locally built images with the documented Helm
# flow, and verifies the post-upgrade expectations:
#
#   - controller, REST, and Flight deployments run the upgraded images
#   - the DataConnectService CR keeps its identity (same uid) and is Ready
#   - connection types created before the upgrade are preserved
#   - data created before the upgrade is still queryable afterwards
#
# Must run after hack/ci/run-tests.sh on the same kind cluster: it reuses the
# locally built images loaded by deploy-dch.sh, the tenant datasources seeded
# for the main e2e suite, and the e2e/.env + e2e/.venv produced by run-e2e.sh.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

DCS_RESOURCE="dataconnectservices.dataconnecthub.opendatahub.io/${CI_DCS_CR_NAME}"
upgrade_base_dir="${CI_TEMP_DIR}/upgrade-base"

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------

preflight() {
    echo "=== Verifying upgrade prerequisites ==="

    local ref_sha
    ref_sha="$(git ls-remote --heads "$CI_UPGRADE_BASE_REPO" "refs/heads/${CI_UPGRADE_BASE_REF}" | cut -f1)"
    if [[ -z "$ref_sha" ]]; then
        echo "ERROR: branch ${CI_UPGRADE_BASE_REF} not found in ${CI_UPGRADE_BASE_REPO}" >&2
        exit 1
    fi

    local image
    for image in \
        "$CI_UPGRADE_BASE_CONTROLLER_IMAGE" \
        "$CI_UPGRADE_BASE_REST_IMAGE" \
        "$CI_UPGRADE_BASE_FLIGHT_IMAGE" \
        "$CI_UPGRADE_BASE_KUBE_RBAC_PROXY_IMAGE"; do
        if ! docker manifest inspect "$image" >/dev/null 2>&1; then
            echo "ERROR: baseline image not pullable: ${image}" >&2
            exit 1
        fi
    done

    if [[ ! -d "$CI_REPO_ROOT/e2e/.venv" || ! -f "$CI_REPO_ROOT/e2e/.env" ]]; then
        echo "ERROR: e2e/.venv or e2e/.env missing — run hack/ci/run-tests.sh first" >&2
        exit 1
    fi

    if ! kubectl get namespace "$CI_SVC_NAMESPACE" >/dev/null 2>&1 ||
        ! kubectl get deployment/dc-controller-manager -n "$CI_CONTROLLER_NAMESPACE" >/dev/null 2>&1; then
        echo "ERROR: DCH is not deployed — run hack/ci/deploy-dch.sh first" >&2
        exit 1
    fi

    echo "Upgrade baseline: ${CI_UPGRADE_BASE_REF} (${ref_sha:0:12}) at ${CI_UPGRADE_BASE_TAG}"
}

# ---------------------------------------------------------------------------
# Teardown the current install
# ---------------------------------------------------------------------------

teardown_current_install() {
    echo "=== Removing the current dc-controller install ==="

    # Delete the CR first, while the controller is still running, so its
    # finalizer can clean up the operand resources.
    kubectl delete "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" \
        --ignore-not-found --wait=true --timeout=180s

    helm uninstall dc-controller -n "$CI_CONTROLLER_NAMESPACE" 2>/dev/null || true

    # Helm never touches crds/ on install or uninstall; remove them so the
    # baseline chart installs its own CRD versions.
    kubectl delete -f "$CI_REPO_ROOT/dc-controller/charts/crds" \
        --ignore-not-found --timeout=180s

    # Fresh service namespace so the baseline version initializes the
    # metadata store schema. The tenant namespace (datasources, e2e SAs,
    # seeded data) and the gateway are intentionally kept.
    kubectl delete namespace "$CI_SVC_NAMESPACE" --wait=true --timeout=180s
    kubectl create namespace "$CI_SVC_NAMESPACE"
}

# ---------------------------------------------------------------------------
# Install the baseline (released) version
# ---------------------------------------------------------------------------

install_baseline() {
    echo "=== Installing baseline ${CI_UPGRADE_BASE_REF} ==="

    rm -rf "$upgrade_base_dir"
    git clone --depth 1 -b "$CI_UPGRADE_BASE_REF" "$CI_UPGRADE_BASE_REPO" "$upgrade_base_dir"

    deploy_system_postgres
    create_service_tls_secrets
    create_gateway_destination_rules

    helm install dc-controller "$upgrade_base_dir/dc-controller/charts" \
        --namespace "$CI_CONTROLLER_NAMESPACE" \
        --set "controllerManager.image.repository=${CI_UPGRADE_BASE_CONTROLLER_IMAGE%:*}" \
        --set "controllerManager.image.tag=${CI_UPGRADE_BASE_TAG}" \
        --set controllerManager.image.pullPolicy=Always \
        --set "relatedImages.restService=${CI_UPGRADE_BASE_REST_IMAGE}" \
        --set "relatedImages.flightService=${CI_UPGRADE_BASE_FLIGHT_IMAGE}" \
        --set "relatedImages.kubeRbacProxy=${CI_UPGRADE_BASE_KUBE_RBAC_PROXY_IMAGE}" \
        --set serviceCA.enabled=true \
        --set serviceCA.configMapName=dch-rest-service-ca

    kubectl rollout status deployment/dc-controller-manager \
        -n "$CI_CONTROLLER_NAMESPACE" --timeout=300s

    echo "=== Creating DataConnectService CR ==="

    kubectl apply -n "$CI_SVC_NAMESPACE" -f - <<EOF
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: ${CI_DCS_CR_NAME}
spec:
  gateway:
    name: ${CI_GATEWAY_NAME}
    namespace: ${CI_GATEWAY_NAMESPACE}
  restService:
    env:
      - name: RUST_LOG
        value: info
  flightService:
    env:
      - name: RUST_LOG
        value: info
EOF

    if ! kubectl wait --for=jsonpath='{.status.phase}'=Ready "$DCS_RESOURCE" \
        -n "$CI_SVC_NAMESPACE" --timeout=180s; then
        kubectl get "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" -o yaml || true
        echo "ERROR: DataConnectService did not become Ready on the baseline version" >&2
        exit 1
    fi

    kubectl rollout status "deployment/${CI_FLIGHT_SERVICE_NAME}" -n "$CI_SVC_NAMESPACE" --timeout=180s
    kubectl rollout status "deployment/${CI_REST_SERVICE_NAME}" -n "$CI_SVC_NAMESPACE" --timeout=180s

    echo "=== Verifying baseline deployment images ==="
    assert_deployment_image dc-controller-manager "$CI_CONTROLLER_NAMESPACE" \
        manager "$CI_UPGRADE_BASE_CONTROLLER_IMAGE"
    assert_deployment_image "$CI_FLIGHT_SERVICE_NAME" "$CI_SVC_NAMESPACE" \
        flight-server "$CI_UPGRADE_BASE_FLIGHT_IMAGE"
    assert_deployment_image "$CI_REST_SERVICE_NAME" "$CI_SVC_NAMESPACE" \
        rest-server "$CI_UPGRADE_BASE_REST_IMAGE"
}

# ---------------------------------------------------------------------------
# Seed data before the upgrade
# ---------------------------------------------------------------------------

seed_pre_upgrade_data() {
    echo "=== Seeding pre-upgrade data ==="

    rm -f "$CI_UPGRADE_STATE_FILE"

    DCH_UPGRADE_PHASE=pre \
        DCH_UPGRADE_STATE_FILE="$CI_UPGRADE_STATE_FILE" \
        "$CI_REPO_ROOT/e2e/.venv/bin/pytest" \
        "$CI_REPO_ROOT/e2e/scenarios/test_upgrade.py" \
        -v -s -k pre_upgrade

    kubectl get "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" \
        -o jsonpath='{.metadata.uid}' > "${CI_TEMP_DIR}/upgrade-dchs-uid"
    kubectl get "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" \
        -o jsonpath='{.metadata.creationTimestamp}' > "${CI_TEMP_DIR}/upgrade-dchs-created"
}

# ---------------------------------------------------------------------------
# Upgrade to the locally built images
# ---------------------------------------------------------------------------

upgrade_to_current() {
    echo "=== Upgrading dc-controller to the current build ==="

    controller_repo="${CI_CONTROLLER_IMAGE%:*}"
    controller_tag="${CI_CONTROLLER_IMAGE##*:}"

    # The single-node Kind cluster cannot run both the old and new
    # one-replica Flight deployments during a rolling update. Use Recreate
    # so Kubernetes stops the old pod before starting the new one.
    kubectl patch deployment "$CI_FLIGHT_SERVICE_NAME" \
        -n "$CI_SVC_NAMESPACE" \
        --type=merge \
        -p '{"spec":{"strategy":{"type":"Recreate","rollingUpdate":null}}}'

    # Helm does not upgrade CRDs shipped in crds/ on helm upgrade; apply the
    # new CRD versions explicitly, as documented in the upgrade guide.
    kubectl apply -f "$CI_REPO_ROOT/dc-controller/charts/crds"

    helm upgrade dc-controller "$CI_REPO_ROOT/dc-controller/charts" \
        --namespace "$CI_CONTROLLER_NAMESPACE" \
        --set "controllerManager.image.repository=${controller_repo}" \
        --set "controllerManager.image.tag=${controller_tag}" \
        --set controllerManager.image.pullPolicy=IfNotPresent \
        --set "relatedImages.restService=${CI_REST_IMAGE}" \
        --set "relatedImages.flightService=${CI_FLIGHT_IMAGE}" \
        --set "relatedImages.kubeRbacProxy=${CI_KUBE_RBAC_PROXY_IMAGE}" \
        --set serviceCA.enabled=true \
        --set serviceCA.configMapName=dch-rest-service-ca

    kubectl rollout status deployment/dc-controller-manager \
        -n "$CI_CONTROLLER_NAMESPACE" --timeout=300s

    if ! kubectl wait --for=jsonpath='{.status.phase}'=Ready "$DCS_RESOURCE" \
        -n "$CI_SVC_NAMESPACE" --timeout=180s; then
        kubectl get "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" -o yaml || true
        echo "ERROR: DataConnectService did not become Ready after the upgrade" >&2
        exit 1
    fi

    kubectl rollout status "deployment/${CI_REST_SERVICE_NAME}" -n "$CI_SVC_NAMESPACE" --timeout=180s
    kubectl rollout status "deployment/${CI_FLIGHT_SERVICE_NAME}" -n "$CI_SVC_NAMESPACE" --timeout=180s
}

# ---------------------------------------------------------------------------
# Verify the upgrade
# ---------------------------------------------------------------------------

verify_upgrade() {
    echo "=== Verifying the upgrade ==="

    local revision status
    revision="$(helm list -n "$CI_CONTROLLER_NAMESPACE" -o json | python3 -c \
        "import json,sys; print([r for r in json.load(sys.stdin) if r['name'] == 'dc-controller'][0]['revision'])")"
    status="$(helm list -n "$CI_CONTROLLER_NAMESPACE" -o json | python3 -c \
        "import json,sys; print([r for r in json.load(sys.stdin) if r['name'] == 'dc-controller'][0]['status'])")"
    [[ "$revision" == "2" ]] || {
        echo "ERROR: expected helm release revision 2, got '${revision}'" >&2
        exit 1
    }
    [[ "$status" == "deployed" ]] || {
        echo "ERROR: expected helm release status deployed, got '${status}'" >&2
        exit 1
    }
    echo "Verified helm release dc-controller revision 2, status deployed"

    echo "=== Verifying upgraded deployment images ==="
    assert_deployment_image dc-controller-manager "$CI_CONTROLLER_NAMESPACE" \
        manager "$CI_CONTROLLER_IMAGE"
    assert_deployment_image "$CI_FLIGHT_SERVICE_NAME" "$CI_SVC_NAMESPACE" \
        flight-server "$CI_FLIGHT_IMAGE"
    assert_deployment_image "$CI_REST_SERVICE_NAME" "$CI_SVC_NAMESPACE" \
        rest-server "$CI_REST_IMAGE"

    echo "=== Verifying the DataConnectService CR identity ==="
    local current_uid current_created expected_uid expected_created
    current_uid="$(kubectl get "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" -o jsonpath='{.metadata.uid}')"
    current_created="$(kubectl get "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" -o jsonpath='{.metadata.creationTimestamp}')"
    expected_uid="$(<"${CI_TEMP_DIR}/upgrade-dchs-uid")"
    expected_created="$(<"${CI_TEMP_DIR}/upgrade-dchs-created")"
    [[ "$current_uid" == "$expected_uid" ]] || {
        echo "ERROR: DataConnectService uid changed across the upgrade: '${expected_uid}' -> '${current_uid}'" >&2
        exit 1
    }
    [[ "$current_created" == "$expected_created" ]] || {
        echo "ERROR: DataConnectService creationTimestamp changed across the upgrade" >&2
        exit 1
    }
    echo "Verified DataConnectService kept its uid (${current_uid})"

    echo "=== Running post-upgrade data checks ==="
    DCH_UPGRADE_PHASE=post \
        DCH_UPGRADE_STATE_FILE="$CI_UPGRADE_STATE_FILE" \
        "$CI_REPO_ROOT/e2e/.venv/bin/pytest" \
        "$CI_REPO_ROOT/e2e/scenarios/test_upgrade.py" \
        -v -s -k post_upgrade
}

# ---------------------------------------------------------------------------
# Failure diagnostics
# ---------------------------------------------------------------------------

on_failure() {
    echo "=== Upgrade test failed — collecting diagnostics ==="
    helm history dc-controller -n "$CI_CONTROLLER_NAMESPACE" 2>&1 || true
    kubectl get "$DCS_RESOURCE" -n "$CI_SVC_NAMESPACE" -o yaml 2>&1 || true
    dump_cluster "$CI_CONTROLLER_NAMESPACE" "$CI_SVC_NAMESPACE"
}
trap on_failure ERR

preflight
teardown_current_install
install_baseline
seed_pre_upgrade_data
upgrade_to_current
verify_upgrade

echo "=== Upgrade test complete ==="
