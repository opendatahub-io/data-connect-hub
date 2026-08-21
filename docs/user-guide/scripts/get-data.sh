#!/bin/bash
set -euo pipefail

. ./common-vars.sh
. ./common-port-forward-rest.sh

CONN_ID="${1}"

pretty_print_json() {
  python3 -c "
import sys, json
raw = sys.stdin.read().strip()
try:
    data = json.loads(raw)
    for k, v in data.items():
        if isinstance(v, str) and len(v) > 80:
            print(f'  {k}: [{len(v)} chars]')
        elif isinstance(v, (dict, list)):
            print(f'  {k}: {json.dumps(v, indent=4)[:200]}')
        else:
            print(f'  {k}: {v}')
except:
    for line in raw.splitlines():
        print(f'  {line}')
" <<< "$1" 2>/dev/null || echo "$1"
}

decode_arrow_data() {
  python3 -c "
import sys, json, base64, struct

raw = sys.stdin.read().strip()
chunks = []
decoder = json.JSONDecoder()
pos = 0
while pos < len(raw):
    s = raw[pos:].lstrip()
    if not s:
        break
    try:
        obj, end = decoder.raw_decode(s)
        chunks.append(obj)
        pos = len(raw) - len(s) + end
    except json.JSONDecodeError:
        break

arrow_msgs = [c for c in chunks if isinstance(c, dict) and 'dataHeader' in c]
if not arrow_msgs:
    for line in raw.splitlines()[:10]:
        print(f'  {line}')
    sys.exit(1)

try:
    import pyarrow as pa
    buf = bytearray()
    for msg in arrow_msgs:
        hdr = base64.b64decode(msg['dataHeader'])
        body = base64.b64decode(msg.get('dataBody', '')) if msg.get('dataBody') else b''
        buf.extend(struct.pack('<I', 0xFFFFFFFF))
        pad = (8 - len(hdr) % 8) % 8
        buf.extend(struct.pack('<i', len(hdr) + pad))
        buf.extend(hdr)
        buf.extend(b'\x00' * pad)
        if body:
            buf.extend(body)
            bp = (8 - len(body) % 8) % 8
            buf.extend(b'\x00' * bp)
    buf.extend(struct.pack('<I', 0xFFFFFFFF))
    buf.extend(struct.pack('<i', 0))
    reader = pa.ipc.open_stream(pa.py_buffer(bytes(buf)))
    table = reader.read_all()
    print(f'  Rows: {table.num_rows}')
    col_widths = {}
    for c in table.column_names:
        vals = [str(v) for v in table.column(c).to_pylist()]
        w = max(len(c), max((len(v) for v in vals), default=0))
        col_widths[c] = min(w, 50)
    header = '  ' + ' | '.join(f'{c:<{col_widths[c]}}' for c in table.column_names)
    print(header)
    print('  ' + '-+-'.join('-' * col_widths[c] for c in table.column_names))
    for i in range(min(table.num_rows, 50)):
        row = []
        for c in table.column_names:
            v = str(table.column(c)[i].as_py())
            if len(v) > 50:
                v = v[:47] + '...'
            row.append(f'{v:<{col_widths[c]}}')
        print('  ' + ' | '.join(row))
    if table.num_rows > 50:
        print(f'  ... ({table.num_rows - 50} more rows)')
except Exception as e:
    print(f'  Arrow decode error: {e}')
    for c in chunks[:3]:
        for k, v in c.items():
            if isinstance(v, str) and len(v) > 80:
                print(f'  {k}: [{len(v)} chars]')
            else:
                print(f'  {k}: {v}')
" <<< "$1" 2>/dev/null || echo "$1"
}

source ./get-token.sh

if [ -z "$user_token" ]; then
  echo "  FAILED: could not obtain token for $SA_NAME (run create-test-user.sh first)"
  cleanup
  exit 1
fi
echo "  Token obtained for $SA_NAME"
echo ""

GRPCURL_VERSION="1.9.1"
PROTO_DIR="/tmp/dch-flight-proto"

mkdir -p "$PROTO_DIR"
if [ ! -f "$PROTO_DIR/Flight.proto" ]; then
  curl -fsSL -o "$PROTO_DIR/Flight.proto" "https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto"
fi
if [ ! -f "$PROTO_DIR/FlightSql.proto" ]; then
  curl -fsSL -o "$PROTO_DIR/FlightSql.proto" "https://raw.githubusercontent.com/apache/arrow/main/format/FlightSql.proto"
fi
echo "  Proto files ready"
echo ""

SQL_QUERY="SELECT * FROM test_prompts"

# Build protobuf commands as base64
CMD_SQLINFO=$(python3 -c "
import base64
type_url = b'type.googleapis.com/arrow.flight.protocol.sql.CommandGetSqlInfo'
cmd_bytes = b'\x0a' + bytes([len(type_url)]) + type_url
print(base64.b64encode(cmd_bytes).decode())
")

CMD_STMT=$(python3 -c "
import base64, sys
def encode_field(fn, data):
    tag = bytes([(fn << 3) | 2])
    length = len(data)
    varint = bytearray()
    while length > 0x7f:
        varint.append((length & 0x7f) | 0x80)
        length >>= 7
    varint.append(length)
    return tag + bytes(varint) + data

query = sys.argv[1].encode()
inner = encode_field(1, query)
type_url = b'type.googleapis.com/arrow.flight.protocol.sql.CommandStatementQuery'
any_bytes = encode_field(1, type_url) + encode_field(2, inner)
print(base64.b64encode(any_bytes).decode())
" "$SQL_QUERY")

echo "  SQL: $SQL_QUERY"
echo "  CMD: grpcurl -insecure -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $TENANT_ID' -H 'x-data-connection-id: $CONN_ID' -d '{\"type\":\"CMD\",\"cmd\":\"<base64>\"}' localhost:$FLIGHT_LOCAL_PORT arrow.flight.protocol.FlightService/GetFlightInfo"
get_info_output=$(grpcurl -insecure  \
  -import-path "$PROTO_DIR" -proto Flight.proto \
  -H "Authorization: Bearer $user_token" \
  -H "x-tenant-id: $TENANT_ID" \
  -H "x-data-connection-id: $CONN_ID" \
  -d "{\"type\":\"CMD\",\"cmd\":\"$CMD_STMT\"}" \
  "localhost:$FLIGHT_LOCAL_PORT" arrow.flight.protocol.FlightService/GetFlightInfo 2>&1) || true
pretty_print_json "$get_info_output"

# Extract ticket 
sql_ticket=$(echo "$get_info_output" | python3 -c "import sys,json; print(json.load(sys.stdin)['endpoint'][0]['ticket']['ticket'])" 2>/dev/null) || true

echo "DoGet for SQL query (expect test_prompts data)"
echo "  SQL: $SQL_QUERY"
if [ -z "$sql_ticket" ]; then
  echo "  SKIPPED  could not extract ticket"
  echo ""
else
  echo "  CMD: grpcurl -insecure -H 'Authorization: Bearer <token>' -H 'x-tenant-id: $TENANT_ID' -H 'x-data-connection-id: $CONN_ID' -d '{\"ticket\":\"<ticket>\"}' localhost:$FLIGHT_LOCAL_PORT arrow.flight.protocol.FlightService/DoGet"
  do_get_output=$(grpcurl -insecure \
    -import-path "$PROTO_DIR" -proto Flight.proto \
    -H "Authorization: Bearer $user_token" \
    -H "x-tenant-id: $TENANT_ID" \
    -H "x-data-connection-id: $CONN_ID" \
    -d "{\"ticket\":\"$sql_ticket\"}" \
    "localhost:$FLIGHT_LOCAL_PORT" arrow.flight.protocol.FlightService/DoGet 2>&1) || true
  decode_arrow_data "$do_get_output"
fi

echo ""
echo "--- Cleanup ---"
cleanup

