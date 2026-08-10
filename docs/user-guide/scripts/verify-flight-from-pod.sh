#!/bin/bash
set -euo pipefail

NAMESPACE="${1:-dch-example}"

echo ""
echo ""
echo "================================== VERIFY FLIGHT SERVICE ============================="
flight_pod=$(oc get po -n "$NAMESPACE" -l app.kubernetes.io/name=flight-service -o jsonpath='{.items[0].metadata.name}')

lsof -ti :50051 2>/dev/null | xargs kill 2>/dev/null || true
sleep 1
echo "  CMD: oc port-forward $flight_pod -n $NAMESPACE 50051:50051"
oc port-forward "$flight_pod" -n "$NAMESPACE" 50051:50051 &
pf_pid=$!
sleep 2

proto_file="/tmp/Flight.proto"
if [ ! -f "$proto_file" ]; then
  echo "  Downloading Flight.proto..."
  curl -sL -o "$proto_file" "https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto"
fi

echo "  CMD: grpcurl -plaintext -import-path /tmp -proto Flight.proto localhost:50051 list"
list_output=$(grpcurl -plaintext -import-path /tmp -proto Flight.proto localhost:50051 list 2>&1)
echo "  RESPONSE: $list_output"

kill $pf_pid 2>/dev/null
wait $pf_pid 2>/dev/null

if ! echo "$list_output" | grep -q "arrow.flight.protocol.FlightService"; then
  echo "FAILED: flight-service did not return FlightService"
  exit 1
fi
echo "PASSED: flight-service responded"

echo ""
echo "  --- Manual testing commands ---"
echo "  flight_pod=\$(oc get po -n $NAMESPACE -l app.kubernetes.io/name=flight-service -o jsonpath='{.items[0].metadata.name}')"
echo "  curl -sL -o /tmp/Flight.proto https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto"
echo "  oc port-forward \$flight_pod -n $NAMESPACE 50051:50051 &"
echo "  grpcurl -plaintext -import-path /tmp -proto Flight.proto localhost:50051 list"
echo "  grpcurl -plaintext -import-path /tmp -proto Flight.proto localhost:50051 describe arrow.flight.protocol.FlightService"
echo "  grpcurl -plaintext -import-path /tmp -proto Flight.proto -d '{}' localhost:50051 arrow.flight.protocol.FlightService/ListFlights"
