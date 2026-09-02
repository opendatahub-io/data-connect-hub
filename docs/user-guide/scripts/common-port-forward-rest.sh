#!/bin/bash
#

pf_pid=""
pff_pid=""
proxy_pid=""

cleanup() {
  if [ -n "$pf_pid" ]; then
    kill $pf_pid 2>/dev/null || true
    wait $pf_pid 2>/dev/null || true
  fi
  if [ -n "$pff_pid" ]; then
    kill $pff_pid 2>/dev/null || true
    wait $pff_pid 2>/dev/null || true
  fi
  if [ -n "$proxy_pid" ]; then
    kill $proxy_pid 2>/dev/null || true
    wait $proxy_pid 2>/dev/null || true
  fi
}

echo "  Finding rest-service pod..."
rest_pod=$(oc get po -n "$INFRA_NAMESPACE" -l app.kubernetes.io/name=rest-service -o jsonpath='{.items[0].metadata.name}' 2>/dev/null) || true
if [ -z "$rest_pod" ]; then
  echo "  FAILED: no rest-service pod found in namespace '$INFRA_NAMESPACE'"
  exit 1
fi
echo "  Pod: $rest_pod"

echo "  Port-forwarding $rest_pod:8080 -> localhost:$REST_LOCAL_PORT..."
lsof -ti :$REST_LOCAL_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc port-forward "pod/$rest_pod" -n "$INFRA_NAMESPACE" "$REST_LOCAL_PORT:8080" &>/dev/null &
export pf_pid=$!
sleep 2

if ! kill -0 $pf_pid 2>/dev/null; then
  echo "  FAILED: port-forward died"
  exit 1
fi
echo "  Port-forward ready (pid=$pf_pid)"
echo ""

echo "  Finding flight-service pod..."
flight_pod=$(oc get po -n "$INFRA_NAMESPACE" -l app.kubernetes.io/name=flight-service -o jsonpath='{.items[0].metadata.name}' 2>/dev/null) || true
if [ -z "$flight_pod" ]; then
  echo "  FAILED: no flight-service pod found in namespace '$INFRA_NAMESPACE'"
  exit 1
fi
echo "  Pod: $flight_pod"

echo "  Port-forwarding $flight_pod:50051 -> localhost:$FLIGHT_LOCAL_PORT..."
lsof -ti :$FLIGHT_LOCAL_PORT 2>/dev/null | xargs kill 2>/dev/null || true
oc port-forward "pod/$flight_pod" -n "$INFRA_NAMESPACE" "$FLIGHT_LOCAL_PORT:8443" &>/dev/null &
pff_pid=$!
sleep 2

if ! kill -0 $pff_pid 2>/dev/null; then
  echo "  FAILED: port-forward died"
  exit 1
fi
echo "  Port-forward ready (pid=$pff_pid)"
echo ""
