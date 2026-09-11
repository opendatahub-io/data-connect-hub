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
CERT_FILE=""

usage() {
    echo "Usage: $0 -u <neo4j-bolt-uri> -n <namespace> \
        [-a admin-password] [--user name] [--pass password] [--ca-cert FILE]"
    exit 1
}

while [[ $# -gt 0 ]]; do
    case $1 in
        -u) NEO4J_URI="$2"; shift 2 ;;
        -n) NAMESPACE="$2"; shift 2 ;;
        -a) ADMIN_PASS="$2"; shift 2 ;;
        --user) READONLY_USER="$2"; shift 2 ;;
        --pass) READONLY_PASS="$2"; shift 2 ;;
        --ca-cert) CERT_FILE="$2"; shift 2 ;;
        -h) usage ;;
        *) usage ;;
    esac
done

[[ -n "$NEO4J_URI" ]] || { echo "error: Neo4j URI is required (-u)" >&2; exit 1; }
[[ -n "$ADMIN_PASS" ]] || { echo "error: Neo4j admin password is required (-a)" >&2; exit 1; }
[[ -n "$READONLY_PASS" ]] || { echo "error: Neo4j read-only password is required (--pass)" >&2; exit 1; }

if [[ -n "$CERT_FILE" ]]; then
    [[ -f "$CERT_FILE" && -r "$CERT_FILE" ]] || {
        echo "error: CA certificate file not found or not readable: $CERT_FILE" >&2
        exit 1
    }
    # Base64-encode the CA so it can be injected into the seed pod as an env var.
    # Preserve PEM line breaks after decoding; removing them would produce an
    # invalid certificate such as "-----BEGIN CERTIFICATE-----MI...".
    NEO4J_CA_B64="$(base64 < "$CERT_FILE" | tr -d '\n')" || {
        echo "error: failed to encode Neo4j CA certificate" >&2
        exit 1
    }
fi

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
    --env="NEO4J_CA_B64=${NEO4J_CA_B64:-}" \
    --command -- sh -c '
set -eu

# When a CA certificate is available, import it into the JDK trust store so
# cypher-shell can verify the server certificate over TLS.
case "$NEO4J_URI" in
    neo4j://*|neo4j+ssc://*)
        ;;
    neo4j+s://*|bolt+s://*)
        if [ -n "${NEO4J_CA_B64:-}" ]; then
            echo "$NEO4J_CA_B64" | base64 -d > /tmp/neo4j-ca.crt || {
                echo "ERROR: failed to decode Neo4j CA certificate" >&2
                exit 1
            }
            [ -s /tmp/neo4j-ca.crt ] || {
                echo "ERROR: decoded Neo4j CA certificate is empty" >&2
                exit 1
            }

            CACERTS=""
            for cand in "${JAVA_HOME:-/opt/neo4j/jdk}/lib/security/cacerts" \
                        /opt/neo4j/jdk/lib/security/cacerts /usr/lib/jvm/*/lib/security/cacerts; do
                if [ -f "$cand" ]; then CACERTS="$cand"; break; fi
            done
            [ -n "$CACERTS" ] || {
                echo "ERROR: Java trust store (cacerts) not found" >&2
                exit 1
            }

            jdk_dir="${CACERTS%/lib/security/cacerts}"
            keytool="${jdk_dir}/bin/keytool"
            if [ ! -x "$keytool" ]; then keytool="$(command -v keytool 2>/dev/null || true)"; fi
            [ -n "$keytool" ] && [ -x "$keytool" ] || {
                echo "ERROR: keytool not found; cannot configure Neo4j TLS verification" >&2
                exit 1
            }

            cp "$CACERTS" /tmp/neo4j-cacerts || {
                echo "ERROR: failed to copy Java trust store" >&2
                exit 1
            }
            local_keytool_output="$("$keytool" -importcert -noprompt -trustcacerts \
                -file /tmp/neo4j-ca.crt -alias neo4j-ca \
                -keystore /tmp/neo4j-cacerts -storepass changeit 2>&1)" || {
                echo "ERROR: failed to import Neo4j CA into temporary trust store:" >&2
                echo "$local_keytool_output" >&2
                exit 1
            }
            export JAVA_TOOL_OPTIONS="-Djavax.net.ssl.trustStore=/tmp/neo4j-cacerts -Djavax.net.ssl.trustStorePassword=changeit${JAVA_TOOL_OPTIONS:+ $JAVA_TOOL_OPTIONS}"
            echo "Imported Neo4j CA into temporary trust store (TLS will verify)"
        else
            echo "Using the system trust store for TLS certificate verification"
        fi
        ;;
    *)
        # Other cypher-shell-supported URI schemes are passed through unchanged.
        ;;
esac

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
