#!/usr/bin/env bash
# Seed PostgreSQL with e2e test data via a Kubernetes pod.
#
# Internal helper: always invoked by run-e2e.sh with command-line flags.
#
# Usage:
#   e2e/scripts/seed-pg-data.sh -u <pg-url> -n <namespace> [-i pg-image]

set -euo pipefail

PG_URL=""
NAMESPACE=""
PG_IMAGE=""

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

[[ -n "$PG_URL" ]] || { echo "error: PostgreSQL URL is required (-u)" >&2; exit 1; }

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
