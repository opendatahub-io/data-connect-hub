# Use the Python SDK REST API on a FIPS cluster

The SDK's REST client runs in the host's FIPS-validated OpenSSL in any FIPS-enabled environment with a RHEL-based Python. This guide runs it from a UBI Python pod: it installs the SDK from PyPI, checks the FIPS status, and lists connection types and connections with `require_fips=True`.

To also query data with Flight SQL, follow [REST and Flight SQL on FIPS-enabled OpenShift AI clusters](fips-rest-flight.md) instead. For background, see the [FIPS dev guide](../dev-guide/fips.md).

# Prerequisites

- A FIPS-enabled OpenShift cluster with Data Connect Hub installed and the gateway configured.
- A tenant namespace and a user with the `dch-read` or `dch-read-write` role in it.
- `oc`, logged in with access to the tenant namespace.
- A RHEL-based Python 3.11 or later that links the system OpenSSL. This guide uses the `ubi9/python-312` image; `ubi9/python-311` works too.

The [getting started guide](../../../docs/user-guide/getting-started-ocp.md) sets all of these up; this guide uses its tenant namespace `dch-tenant-a` and user `dch-test-user`.

# Set Variables

```sh
TENANT_NAMESPACE=dch-tenant-a
SA_NAME=dch-test-user
POD=dch-sdk
GW_HOST=$(oc get route data-science-gateway -n openshift-ingress -o jsonpath='{.spec.host}')
echo GW_HOST=$GW_HOST
```

# Start a UBI Python Pod

```sh
oc run $POD -n $TENANT_NAMESPACE --restart=Never \
  --image=registry.access.redhat.com/ubi9/python-312 \
  --command -- sleep infinity
oc wait pod/$POD -n $TENANT_NAMESPACE --for=condition=Ready --timeout=300s
```

You should see:

```
pod/dch-sdk created
pod/dch-sdk condition met
```

You may also see a `PodSecurity` warning; the pod still starts.

# Install the SDK

The REST install has no cryptography of its own, so it can come from PyPI:

```sh
oc exec -n $TENANT_NAMESPACE $POD -- pip install -q "data-connect-hub>=0.3.0"
```

# Copy the Token and CA Into the Pod

Create a token for the user, and copy the cluster's ingress CA, which signs the gateway certificate by default:

```sh
oc create token $SA_NAME -n $TENANT_NAMESPACE --duration=1h \
  | oc exec -i -n $TENANT_NAMESPACE $POD -- sh -c 'cat > /tmp/dch.token'
oc get configmap default-ingress-cert -n openshift-config-managed -o jsonpath='{.data.ca-bundle\.crt}' \
  | oc exec -i -n $TENANT_NAMESPACE $POD -- sh -c 'cat > /tmp/ingress-ca.crt'
```

If your gateway uses a certificate from a public CA, skip the CA and leave `DCH_CA_CERT` unset in the next steps.

# Check FIPS

```sh
oc exec -n $TENANT_NAMESPACE $POD -- python -c "from data_connect_hub import fips_status; print(fips_status())"
```

You should see:

```
FipsStatus(host_fips=True, rest_tls='fips', flight_system_openssl=None, pyarrow_version=None, pyarrow_error=None)
```

`host_fips=True` means the node runs in FIPS mode, and `rest_tls='fips'` means REST TLS uses the system OpenSSL with the FIPS provider enforced. `flight_system_openssl` is `None` because this REST-only install has no pyarrow.

# Call the REST API

List the connection types and connections in the tenant. `require_fips=True` makes the client fail if the FIPS checks above don't pass:

```sh
oc exec -i -n $TENANT_NAMESPACE $POD -- env \
  DCH_HOST="$GW_HOST:443" DCH_TENANT_ID="$TENANT_NAMESPACE" DCH_CA_CERT=/tmp/ingress-ca.crt \
  python - <<'PY'
import os
import sys

from data_connect_hub import DataConnectClient, DCHError

try:
    with DataConnectClient(
        endpoint=os.environ["DCH_HOST"],
        token=open("/tmp/dch.token").read().strip(),
        tenant_id=os.environ["DCH_TENANT_ID"],
        ca_cert=os.getenv("DCH_CA_CERT"),
        require_fips=True,
    ) as client:
        for connection_type in client.list_connection_types():
            print(f"type        {connection_type.name:<20} provider={connection_type.provider}")
        for connection in client.list_connections():
            print(f"connection  {connection.name:<20} state={connection.status.state}")
except DCHError as error:
    sys.exit(f"Unable to call Data Connect Hub: {error}")
PY
```

You should see something similar to this:

```
type        test-postgres-1      provider=postgres
connection  test-pg-conn-1       state=ready
```

# Clean Up

```sh
oc delete pod $POD -n $TENANT_NAMESPACE
```
