#!/usr/bin/env bash
# Seed PostgreSQL with e2e test data via a Kubernetes pod.
#
# Internal helper: always invoked by run-e2e.sh with command-line flags.
#
# Usage:
#   e2e/scripts/seed-pg-data.sh -u <pg-url> -n <namespace> [-i pg-image] [-c <ca-cert-file>]

set -euo pipefail

PG_URL=""
NAMESPACE=""
PG_IMAGE=""
CA_CERT_PATH=""     # Optional path to a CA certificate file. When PG_URL uses
                    # sslmode=verify-ca/verify-full, it is base64-encoded and made
                    # available inside the seed pod (decoded to a file, then exposed to
                    # libpq via PGSSLROOTCERT). If none is provided, we refuse rather
                    # than downgrade the URL to an unauthenticated one.

usage() {
    echo "Usage: $0 -u <pg-url> -n <namespace> [-i pg-image] [-c <ca-cert-file>]" >&2
    exit 1
}

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        -u)   shift; PG_URL=${1-} ;;
        -n)   shift; NAMESPACE=${1-} ;;
        -i)   shift; PG_IMAGE=${1-} ;;
        -c)   shift; CA_CERT_PATH=${1-} ;;
        -h|--help) usage ;;
        *) echo "ERROR: unknown option: $1" >&2; exit 1 ;;
    esac
    shift
done

[[ -n "$PG_URL" ]] || { echo "error: PostgreSQL URL is required (-u)" >&2; exit 1; }

# Preserve certificate verification. Do NOT downgrade sslmode=verify-ca/
# verify-full to sslmode=require. sslmode=require only encrypts the connection;
# it does not authenticate the PostgreSQL server (CWE-295), so an in-cluster
# MITM (compromised DNS/Service/route) could redirect the seed pod and replay its
# credentials. When PG_URL requests certificate verification, the CA certificate
# (-c) must be provided and exposed inside the pod via PGSSLROOTCERT; if none is
# provided, fail before starting the pod rather than weakening the connection.
SEED_URL="$PG_URL"
CA_B64=""
if [[ "${PG_URL}" =~ sslmode=verify-(ca|full) ]]; then
    if [[ -z "${CA_CERT_PATH}" || ! -f "${CA_CERT_PATH}" ]]; then
        echo "ERROR: $0: PG_URL uses sslmode=verify-ca/verify-full but no CA certificate " \
             "was provided with -c; refusing to weaken it to sslmode=require." >&2
        exit 1
    fi
    CA_B64="$(base64 -i "$CA_CERT_PATH" | tr -d '\n')"
fi

POD_NAME="e2e-pg-seed"
# Proxy env only when we actually have a CA to verify with, so sslmode=require URLs
# keep working unauthenticated (no PGSSLROOTCERT -> libpq honors sslmode).
POD_ENVS=()
if [[ -n "${CA_B64:-}" ]]; then
    POD_ENVS+=(--env="E2E_PG_CA_B64=${CA_B64}")
    POD_ENVS+=(--env="PGSSLROOTCERT=/tmp/postgresql-ca.crt")
fi

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
kubectl run "$POD_NAME" -n "$NAMESPACE" \
    --image="$PG_IMAGE" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    ${POD_ENVS[@]+"${POD_ENVS[@]}"} \
    --env="PGURI=${SEED_URL}" \
    --command -- sh -c '
        if [ -n "${E2E_PG_CA_B64:-}" ]; then
            printf "%s" "$E2E_PG_CA_B64" | base64 -d > /tmp/postgresql-ca.crt
        fi
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
