#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"
SA_NAME="${SA_NAME:-dch-test-user}"

echo "--- Obtaining token ---"
PROXY_PORT=18082
lsof -ti :$PROXY_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc proxy --port=$PROXY_PORT &>/dev/null &
proxy_pid=$!
sleep 1

user_token=$(curl -s -X POST "http://127.0.0.1:${PROXY_PORT}/api/v1/namespaces/${NAMESPACE}/serviceaccounts/${SA_NAME}/token" \
  -H "Content-Type: application/json" \
  -d "{\"apiVersion\":\"authentication.k8s.io/v1\",\"kind\":\"TokenRequest\",\"spec\":{\"audiences\":[\"https://kubernetes.default.svc\"],\"expirationSeconds\":3600}}" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['status']['token'])" 2>/dev/null) || true

kill $proxy_pid 2>/dev/null || true
wait $proxy_pid 2>/dev/null || true
proxy_pid=""
echo $user_token
