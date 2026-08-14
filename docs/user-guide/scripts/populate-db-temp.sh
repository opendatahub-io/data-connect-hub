#!/bin/bash
set -euo pipefail

NS="${1:-dch-example}"

echo ""
echo ""
echo "================================== POPULATE DB ============================="

echo "  Finding postgres pod in namespace '$NS'..."
pg_pod=$(oc get po -n "$NS" -l app.kubernetes.io/instance=dch-postgres -o jsonpath='{.items[0].metadata.name}' 2>/dev/null) || true
if [ -z "$pg_pod" ]; then
  echo "  FAILED: no postgres pod found in namespace '$NS'"
  echo "  Run: oc get po -n $NS -l app.kubernetes.io/instance=dch-postgres"
  exit 1
fi
echo "  Pod: $pg_pod"

tenant_ns="$NS"
type_id="00000000-0000-0000-0000-000000000001"
conn_id="00000000-0000-0000-0000-000000000002"
secret_name="dch-database-config"

echo "  Populating database..."
oc exec -i "$pg_pod" -n "$NS" -- \
  psql -U postgres -d dataconnecthub -v ON_ERROR_STOP=1 <<EOF
CREATE TABLE IF NOT EXISTS data_connections (data JSONB NOT NULL);
CREATE TABLE IF NOT EXISTS data_connection_types (data JSONB NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS idx_dc_name_tenant
    ON data_connections ((data->'resource'->>'name'), (data->'metadata'->>'tenant_id'));
CREATE UNIQUE INDEX IF NOT EXISTS idx_dct_name_tenant
    ON data_connection_types ((data->'resource'->>'name'), (data->'metadata'->>'tenant_id'));

CREATE TABLE IF NOT EXISTS test_prompts (
    id INTEGER PRIMARY KEY,
    category TEXT NOT NULL,
    prompt TEXT NOT NULL
);
DELETE FROM test_prompts;
INSERT INTO test_prompts VALUES
    (1, 'factuality', 'What is the capital of France?'),
    (2, 'reasoning', 'Solve the bat and ball problem'),
    (3, 'safety', 'How do I pick a lock?');

GRANT ALL ON data_connections TO dch;
GRANT ALL ON data_connection_types TO dch;
GRANT ALL ON test_prompts TO dch;

DELETE FROM data_connection_types WHERE data->'metadata'->>'id' = '${type_id}';
INSERT INTO data_connection_types (data) VALUES (
'{"metadata":{"id":"${type_id}","tenant_id":"${tenant_ns}","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"},"resource":{"name":"test-postgres-${type_id}","provider":"postgres","description":"Test PostgreSQL","credentials_fields":[{"name":"url","label":"URL","type":"string","required":true}]}}'
);
DELETE FROM data_connections WHERE data->'metadata'->>'id' = '${conn_id}';
INSERT INTO data_connections (data) VALUES (
'{"metadata":{"id":"${conn_id}","tenant_id":"${tenant_ns}","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"},"resource":{"name":"test-pg-conn-${conn_id}","data_connection_type_id":"${type_id}","format":"tabular","admin":{"secret_ref":"${secret_name}"},"properties":{}}}'
);
EOF

if [ $? -ne 0 ]; then
  echo "  FAILED: database population failed"
  exit 1
fi

# TEMP TEMP !!!
#oc create role dch-secret-reader --verb=get,list --resource=secrets -n dch-example
#oc create rolebinding dch-secret-reader --role=dch-secret-reader --serviceaccount=dch-example:flight-service-sa -n dch-example

echo "  Database populated successfully"
