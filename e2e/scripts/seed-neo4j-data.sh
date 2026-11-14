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

POD_NAME="dch-e2e-neo4j-seed"

# The pod runs to completion. A single `kubectl wait` for the Succeeded phase
# would need one budget to cover the image pull, the container start, and the
# in-pod readiness loop (60 attempts x (2s sleep + JVM startup)), and a
# jsonpath wait cannot fail fast on a Failed pod — it burns the whole timeout
# even when the script exited immediately. Split the wait: first wait for the
# container to start (pull included), then for a terminal phase, each with its
# own budget. Override via environment variables for slow registries.
SEED_START_TIMEOUT="${SEED_START_TIMEOUT:-300}"   # seconds: image pull + container start
SEED_RUN_TIMEOUT="${SEED_RUN_TIMEOUT:-600}"       # seconds: in-pod seed script

pod_phase() {
    kubectl get pod "$POD_NAME" -n "$NAMESPACE" \
        -o jsonpath='{.status.phase}' 2>/dev/null || true
}

# Return 0 once the pod reaches one of the comma-separated phases, 1 on timeout.
wait_for_pod_phase() {
    local accepted="$1" timeout="$2" waited=0 phase
    while :; do
        phase="$(pod_phase)"
        if [[ -n "$phase" && ",$accepted," == *",$phase,"* ]]; then
            return 0
        fi
        if (( waited >= timeout )); then
            return 1
        fi
        sleep 5
        waited=$((waited + 5))
    done
}

# Print the pod events (image pull, scheduling) and its full logs so the real
# failure cause is visible; used by every failure path below.
dump_seed_pod_diagnostics() {
    local stage="$1"
    echo "--- pod events and status ($stage) ---" >&2
    kubectl describe pod "$POD_NAME" -n "$NAMESPACE" >&2 || true
    echo "--- pod logs ($stage) ---" >&2
    kubectl logs "$POD_NAME" -n "$NAMESPACE" >&2 || true
}

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

# Flatten the multi-line cypher seed into a single YAML double-quoted scalar
# (\n escapes), and stage the seed script in a temp file. Both are computed
# before the manifest heredoc on purpose: this bash (3.2 on macOS) parses a
# $(cat <<'X' ...) substitution inside an unquoted heredoc as shell code, so
# an unbalanced ")" in the script body would truncate it. The manifest heredoc
# below therefore only contains plain variables and one simple $(sed ...).
CYPHER_ONELINE="$(printf '%s' "$CYPHER_SEED" | sed -e 's/\\/\\\\/g' -e 's/$/\\n/' | tr -d '\n')"
SEED_TMP="$(mktemp)"
trap 'rm -f "$SEED_TMP"' EXIT
cat > "$SEED_TMP" <<'SEED'
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
# Keep the last cypher-shell error so a readiness failure reports its actual
# cause (authentication, TLS, connectivity) instead of a bare "not reachable".
last_error=/tmp/neo4j-cypher-last-error.txt
for i in $(seq 1 60); do
  if cs "RETURN 1" >"$last_error" 2>&1; then
    ready=1; break
  fi
  echo "Neo4j not ready yet (attempt $i/60)"
  sleep 2
done
if [ "$ready" -ne 1 ]; then
  echo "ERROR: Neo4j not reachable at $NEO4J_URI after 60 attempts" >&2
  echo "--- last cypher-shell error ---" >&2
  cat "$last_error" >&2
  exit 1
fi

echo "Cleaning previous test data"
cs "MATCH (n) WHERE n.dch_e2e = true DETACH DELETE n;"

echo "Inserting test data"
cs "$CYPHER_SEED" || { echo "Failed to insert test data" >&2; exit 1; }

echo "Creating read-only user: $READONLY_USER"
cs "DROP USER $READONLY_USER IF EXISTS;" 2>/dev/null || true
cs "CREATE USER $READONLY_USER SET PASSWORD '${READONLY_PASS}' SET PASSWORD CHANGE NOT REQUIRED;" || { echo "Failed to create read-only user" >&2; exit 1; }
if cs "GRANT ROLE reader TO $READONLY_USER;" 2>/dev/null; then
  echo "Granted reader role to $READONLY_USER"
else
  echo "WARNING: GRANT ROLE not supported (Community Edition) — user has full access"
fi

echo "Neo4j seed data and read-only user created successfully"
SEED

# Use OpenShift's namespace-assigned base UID when available; fall back to the
# image default (7474 = neo4j) on non-OpenShift clusters where kubelet needs a
# numeric UID to verify runAsNonRoot.
RUN_AS_USER=$(kubectl get namespace "$NAMESPACE" \
    -o jsonpath='{.metadata.annotations.openshift\.io/sa\.scc\.uid-range}' 2>/dev/null \
    | cut -d/ -f1)
: "${RUN_AS_USER:=7474}"

kubectl delete pod "$POD_NAME" -n "$NAMESPACE" --ignore-not-found >/dev/null 2>&1 || true

kubectl apply -n "$NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: ${POD_NAME}
spec:
  restartPolicy: Never
  # OpenShift namespaces enforce the PodSecurity "restricted" profile, which
  # rejects pods that do not set an explicit securityContext.
  securityContext:
    runAsNonRoot: true
    seccompProfile:
      type: RuntimeDefault
  containers:
    - name: seed
      image: neo4j:5-community
      imagePullPolicy: IfNotPresent
      securityContext:
        allowPrivilegeEscalation: false
        capabilities:
          drop: ["ALL"]
        runAsNonRoot: true
        runAsUser: ${RUN_AS_USER}
        seccompProfile:
          type: RuntimeDefault
      env:
        - name: NEO4J_URI
          value: "${NEO4J_URI}"
        - name: ADMIN_PASS
          value: "${ADMIN_PASS}"
        - name: CYPHER_SEED
          value: "${CYPHER_ONELINE}"
        - name: READONLY_USER
          value: "${READONLY_USER}"
        - name: READONLY_PASS
          value: "${READONLY_PASS}"
        - name: NEO4J_CA_B64
          value: "${NEO4J_CA_B64:-}"
      command:
        - /bin/sh
        - -c
        - |
$(sed 's/^/          /' "$SEED_TMP")
EOF

if ! wait_for_pod_phase "Running,Succeeded,Failed" "$SEED_START_TIMEOUT"; then
    echo "ERROR: seed pod '$POD_NAME' did not start within ${SEED_START_TIMEOUT}s " \
         "(phase: $(pod_phase); slow image pull or scheduling issue?)" >&2
    dump_seed_pod_diagnostics "start"
    exit 1
fi

# The pod reached Running (or already finished); Succeeded/Failed short-circuit
# the stage below. The seed script gets its own budget, separate from the pull.
if ! wait_for_pod_phase "Succeeded,Failed" "$SEED_RUN_TIMEOUT"; then
    echo "ERROR: seed pod '$POD_NAME' did not finish within ${SEED_RUN_TIMEOUT}s " \
         "(phase: $(pod_phase); still waiting for Neo4j?)" >&2
    dump_seed_pod_diagnostics "run"
    exit 1
fi

if [[ "$(pod_phase)" != "Succeeded" ]]; then
    echo "ERROR: seed pod '$POD_NAME' failed" >&2
    dump_seed_pod_diagnostics "run"
    exit 1
fi
# Leave the completed seed pod in place for inspection; the next run deletes
# and recreates it.
