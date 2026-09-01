#!/usr/bin/env bash
# Seed Neo4j with test data and create a read-only user via a Kubernetes pod.
#
# The script creates a separate Neo4j user for the connector. On Enterprise
# Edition it also grants the built-in "reader" role so the connector is
# read-only. On Community Edition GRANT ROLE is not supported — the user
# will have full access and a warning is printed.
#
# Internal helper: always invoked by run-e2e.sh with command-line flags.
#
# Usage:
#   e2e/scripts/seed-neo4j-data.sh -u <neo4j-bolt-uri> -n <namespace> \
#       [-a admin-password] [--user name] [--pass password]

set -euo pipefail

NEO4J_URI=""
NAMESPACE=""
ADMIN_PASS=""
READONLY_USER="dch_reader"
READONLY_PASS=""

usage() {
    echo "Usage: $0 -u <neo4j-bolt-uri> [-n namespace] [-a admin-password] [--user name] [--pass password]"
    exit 1
}

while [[ $# -gt 0 ]]; do
    case $1 in
        -u) NEO4J_URI="$2"; shift 2 ;;
        -n) NAMESPACE="$2"; shift 2 ;;
        -a) ADMIN_PASS="$2"; shift 2 ;;
        --user) READONLY_USER="$2"; shift 2 ;;
        --pass) READONLY_PASS="$2"; shift 2 ;;
        -h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$NEO4J_URI" ]] || { echo "error: Neo4j URI is required (-u)" >&2; exit 1; }
[[ -n "$ADMIN_PASS" ]] || { echo "error: Neo4j admin password is required (-a)" >&2; exit 1; }
[[ -n "$READONLY_PASS" ]] || { echo "error: Neo4j read-only password is required (--pass)" >&2; exit 1; }

POD_NAME="e2e-neo4j-seed"

CYPHER_SEED=$(cat <<'CYPHER'
CREATE (tokyo:City {name: 'Tokyo', country: 'Japan', population: 13960000, dch_e2e: true}),
       (london:City {name: 'London', country: 'United Kingdom', population: 8982000, dch_e2e: true}),
       (paris:City {name: 'Paris', country: 'France', population: 2161000, dch_e2e: true}),
       (nyc:City {name: 'New York', country: 'United States', population: 8336000, dch_e2e: true}),
       (berlin:City {name: 'Berlin', country: 'Germany', population: 3645000, dch_e2e: true}),
       (alice:Person {name: 'Alice', age: 30, dch_e2e: true}),
       (bob:Person {name: 'Bob', age: 25, dch_e2e: true}),
       (carol:Person {name: 'Carol', age: 35, dch_e2e: true}),
       (alice)-[:LIVES_IN]->(tokyo),
       (bob)-[:LIVES_IN]->(london),
       (carol)-[:LIVES_IN]->(paris),
       (alice)-[:KNOWS]->(bob),
       (bob)-[:KNOWS]->(carol),
       (tokyo)-[:FLIGHT_TO {distance_km: 9571}]->(london),
       (london)-[:FLIGHT_TO {distance_km: 344}]->(paris),
       (paris)-[:FLIGHT_TO {distance_km: 5837}]->(nyc),
       (nyc)-[:FLIGHT_TO {distance_km: 6385}]->(berlin),
       (berlin)-[:FLIGHT_TO {distance_km: 8918}]->(tokyo);
CYPHER
)

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
kubectl run "$POD_NAME" -n "$NAMESPACE" \
    --image="neo4j:5-community" \
    --image-pull-policy=IfNotPresent \
    --restart=Never \
    --env="NEO4J_URI=${NEO4J_URI}" \
    --env="ADMIN_PASS=${ADMIN_PASS}" \
    --env="CYPHER_SEED=${CYPHER_SEED}" \
    --env="READONLY_USER=${READONLY_USER}" \
    --env="READONLY_PASS=${READONLY_PASS}" \
    --command -- sh -c '
cs() { cypher-shell -a "$NEO4J_URI" -u neo4j -p "$ADMIN_PASS" "$@"; }

ready=0
for i in $(seq 1 60); do
  if cs "RETURN 1" >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 2
done
[ "$ready" -eq 1 ] || { echo "Neo4j not reachable at $NEO4J_URI" >&2; exit 1; }

echo "Cleaning previous test data"
cs "MATCH (n) WHERE n.dch_e2e = true DETACH DELETE n;"

echo "Inserting test data"
cs "$CYPHER_SEED" || { echo "Failed to insert test data" >&2; exit 1; }

echo "Creating read-only user: $READONLY_USER"
cs "DROP USER $READONLY_USER IF EXISTS;" 2>/dev/null || true
cs "CREATE USER $READONLY_USER SET PASSWORD '\''${READONLY_PASS}'\'' SET PASSWORD CHANGE NOT REQUIRED;" || { echo "Failed to create read-only user" >&2; exit 1; }
if cs "GRANT ROLE reader TO $READONLY_USER;" 2>/dev/null; then
  echo "Granted reader role to $READONLY_USER"
else
  echo "WARNING: GRANT ROLE not supported (Community Edition) — user has full access"
fi

echo "Neo4j seed data and read-only user created successfully"
'

kubectl wait --for=jsonpath='{.status.phase}'=Succeeded "pod/$POD_NAME" \
    -n "$NAMESPACE" --timeout=120s || {
    kubectl logs "$POD_NAME" -n "$NAMESPACE" --tail=20 || true
    echo "ERROR: seed pod '$POD_NAME' failed" >&2
    exit 1
}
kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true
