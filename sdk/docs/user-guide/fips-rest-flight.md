# Use the Python SDK REST API and Flight SQL on FIPS-enabled OpenShift AI clusters

Flight SQL queries run in the host's FIPS-validated OpenSSL only with pyarrow from the Red Hat AI Python index, which runs in RHOAI workbench images. This guide starts a pod from the minimal workbench image, installs the SDK, checks the FIPS status, and queries a table with `require_fips=True`.

If you only need REST, a plain UBI Python pod is enough; see [REST on a FIPS cluster](fips-rest.md). For background, see the [FIPS dev guide](../dev-guide/fips.md).

# Prerequisites

- A FIPS-enabled OpenShift cluster with Red Hat OpenShift AI and Data Connect Hub installed, and the gateway configured.
- A tenant namespace, a user with the `dch-read` or `dch-read-write` role in it, and a tabular connection.
- `oc`, logged in with access to the tenant namespace.
- A **Python 3.12** workbench image. The Red Hat AI Python index publishes pyarrow only for Python 3.12, so Flight SQL doesn't run in the host's FIPS module with other Python versions. This guide uses the minimal Python 3.12 workbench image.

The [getting started guide](../../../docs/user-guide/getting-started-ocp.md) sets all of these up; this guide uses its tenant namespace `dch-tenant-a`, user `dch-test-user`, connection `test-pg-conn-1` and table `test_prompts`.

# Set Variables

```sh
TENANT_NAMESPACE=dch-tenant-a
SA_NAME=dch-test-user
CONN_NAME=test-pg-conn-1
POD=dch-sdk
GW_HOST=$(oc get route data-science-gateway -n openshift-ingress -o jsonpath='{.spec.host}')
echo GW_HOST=$GW_HOST
```

# Start a Workbench Pod

Start a pod from the minimal Python 3.12 workbench image. It ships the system libraries that Red Hat pyarrow needs, and points pip at the Red Hat AI Python index.

Take the image reference from the `s2i-minimal-notebook` imagestream that RHOAI installs, so it always matches the installed RHOAI version. The imagestream marks the current image as recommended:

```sh
WB_IMAGE=$(oc get imagestream s2i-minimal-notebook -n redhat-ods-applications \
  -o jsonpath='{.spec.tags[?(@.annotations.opendatahub\.io/workbench-image-recommended=="true")].from.name}')
echo WB_IMAGE=$WB_IMAGE
```

You should see something similar to this:

```
WB_IMAGE=registry.redhat.io/rhoai/odh-workbench-jupyter-minimal-cpu-py312-rhel9@sha256:16027a25...
```

The image's entrypoint starts JupyterLab, so the pod runs `sleep infinity` instead and keeps running until you delete it:

```sh
oc run $POD -n $TENANT_NAMESPACE --restart=Never \
  --image=$WB_IMAGE \
  --command -- sleep infinity
oc wait pod/$POD -n $TENANT_NAMESPACE --for=condition=Ready --timeout=600s
```

You should see:

```
pod/dch-sdk created
pod/dch-sdk condition met
```

The first start can take a few minutes while the node pulls the image. You may also see a `PodSecurity` warning; the pod still starts.

**Note**: If you already have an RHOAI dashboard workbench with a Red Hat Python 3.12 image, you can use it instead of creating a workbench pod. Open a terminal in the workbench and run the commands below without the `oc exec -n $TENANT_NAMESPACE $POD --` prefix.

# Install the SDK

pip in the workbench image installs from the Red Hat AI Python index, including Red Hat pyarrow. Don't add PyPI as an extra index:

```sh
oc exec -n $TENANT_NAMESPACE $POD -- pip install -q "data-connect-hub[flight]>=0.3.0"
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

You should see something similar to this:

```
FipsStatus(host_fips=True, rest_tls='fips', flight_system_openssl=True, pyarrow_version='25.0.1', pyarrow_error=None)
```

`flight_system_openssl=True` means Flight SQL uses the system OpenSSL. If it's `False`, pyarrow came from PyPI; if `pyarrow_error` is set, pyarrow can't load in this image.

# Query Data

Look up the connection with REST, then query its table with Flight SQL:

```sh
oc exec -i -n $TENANT_NAMESPACE $POD -- env \
  DCH_HOST="$GW_HOST:443" DCH_TENANT_ID="$TENANT_NAMESPACE" DCH_CA_CERT=/tmp/ingress-ca.crt CONN_NAME="$CONN_NAME" \
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
        connection = next(c for c in client.list_connections() if c.name == os.environ["CONN_NAME"])
        print(f"connection {connection.name} ({connection.id}) state={connection.status.state}\n")
        table = client.read("SELECT * FROM test_prompts LIMIT 10", connection_id=connection.id)
        print(table.to_pandas().to_string(index=False))
except StopIteration:
    sys.exit(f"No connection named {os.environ['CONN_NAME']!r}")
except DCHError as error:
    sys.exit(f"Unable to query Data Connect Hub: {error}")
PY
```

You should see something similar to this:

```
connection test-pg-conn-1 (6f1c2a4e-...) state=ready

 id   category                         prompt
  1 factuality What is the capital of France?
  2  reasoning Solve the bat and ball problem
  3     safety          How do I pick a lock?
```

# Clean Up

```sh
oc delete pod $POD -n $TENANT_NAMESPACE
```
