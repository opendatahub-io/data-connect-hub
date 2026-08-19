#!/usr/bin/env bash
# Seed PostgreSQL with e2e test data via a Kubernetes pod.
#
# Usage:
#   e2e/scripts/seed-pg-data.sh -u <pg-url> -n <namespace>
#
# Environment overrides (command-line flags take precedence):
#   PG_INTERNAL_URL         PostgreSQL connection URL (required)
#   DCH_SERVICE_NAMESPACE   Namespace for seed pod   (default: dch)
#   DCH_POSTGRES_IMAGE      PostgreSQL client image   (default: docker.io/library/postgres:16)

set -euo pipefail

PG_URL="${PG_INTERNAL_URL:-}"
NAMESPACE="${DCH_SERVICE_NAMESPACE:-dch}"
PG_IMAGE="${DCH_POSTGRES_IMAGE:-docker.io/library/postgres:16}"

usage() {
    echo "Usage: $0 -u <pg-url> -n <namespace> [-i pg-image]"
    exit 1
}

while getopts "u:n:i:h" opt; do
    case $opt in
        u) PG_URL="$OPTARG" ;;
        n) NAMESPACE="$OPTARG" ;;
        i) PG_IMAGE="$OPTARG" ;;
        h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$PG_URL" ]] || { echo "error: PostgreSQL URL is required (-u or PG_INTERNAL_URL)" >&2; exit 1; }

POD_NAME="e2e-pg-seed"

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
kubectl run "$POD_NAME" -n "$NAMESPACE" \
    --image="$PG_IMAGE" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    --env="PGURI=${PG_URL}" \
    --command -- sh -c '
psql "$PGURI" -v ON_ERROR_STOP=1 <<SQL
CREATE SCHEMA IF NOT EXISTS dch_e2e;
DROP TABLE IF EXISTS dch_e2e.cities;
CREATE TABLE dch_e2e.cities (
    id   SERIAL PRIMARY KEY,
    name TEXT    NOT NULL,
    country TEXT NOT NULL,
    population INTEGER NOT NULL
);
INSERT INTO dch_e2e.cities (name, country, population) VALUES
    ('"'"'Tokyo'"'"',  '"'"'Japan'"'"',          13960000),
    ('"'"'London'"'"', '"'"'United Kingdom'"'"',  8982000),
    ('"'"'Paris'"'"',  '"'"'France'"'"',          2161000);
SQL
'

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "ERROR: seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
