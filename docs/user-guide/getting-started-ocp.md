
In this step-by-step guide, you'll install DCH on an OpenShift cluster, set up a tenant with a Postgres data source, create connections through the DCH REST API, and query the data using the DCH Python SDK.

# Prerequisites

Before you begin, make sure you have the following:

* An OpenShift cluster (version 4.20 or later)  
* Either the oc command  
* Cluster admin permissions  
* [Helm](https://helm.sh/docs/intro/install/), jq,  curl, grpcurl, python3.11+ installed. Install grpcurl with go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest  
* RHOAI installed and DataScienceClusterInitialization is ready:

  ```sh
    oc get dsci -A
  ```

	You should see:

  ```
  NAME           AGE   PHASE   CREATED AT
    default-dsci   83d   Ready   2026-05-08T12:41:52Z
  ```

* DCH uses a Postgres database to store its meta data. As a cluster admin, you can create a Postgres database as follows:  
  * Install the Postgres operator:


    ```sh

    oc apply -f - <<'EOF'
    apiVersion: operators.coreos.com/v1alpha1
    kind: Subscription
    metadata:
      name: cloudnative-pg
      namespace: openshift-operators
    spec:
      channel: stable-v1
      name: cloudnative-pg
      source: certified-operators
      sourceNamespace: openshift-marketplace
      installPlanApproval: Automatic
    EOF
    ```


  * Check the operator to make sure it's Succeeded as follows:

    ```sh
    oc get csv -n openshift-operators -l operators.coreos.com/cloudnative-pg.openshift-operators=
    ```

	You should see:

    ```
    NAME                     DISPLAY         VERSION   REPLACES                 PHASE
        cloudnative-pg.v1.30.0   CloudNativePG   1.30.0    cloudnative-pg.v1.29.2   Succeeded
    ```

* HTTP/2 enabled on the ingress
  
  gRPC requires HTTP/2 via Application-Level Protocol Negotiation (ALPN), which OpenShift's router disables by default. 

  On a managed OpenShift cluster (ROSA/OSD), enable it on the IngressController:

  ```sh
  oc annotate ingresscontroller default -n openshift-ingress-operator \
    ingress.operator.openshift.io/default-enable-http2=true --overwrite
  ```

  On other clusters enable it using:

  ```sh
  oc annotate ingresses.config/cluster \
    ingress.operator.openshift.io/default-enable-http2=true --overwrite
  ```

* Give the gateway's Route a dedicated TLS certificate


  HAProxy refuses ALPN on routes using the router's shared default certificate (to prevent connection coalescing across hostnames), so enabling HTTP/2 alone is not enough. Reuse the cluster's trusted wildcard cert:

  ```sh
  INGRESS_CERT=$(oc get ingresscontroller default -n openshift-ingress-operator -o jsonpath='{.spec.defaultCertificate.name}')

  oc get secret $INGRESS_CERT -n openshift-ingress \
    -o jsonpath='{.data.tls\.crt}' | base64 -d > /tmp/route.crt
  oc get secret $INGRESS_CERT -n openshift-ingress \
    -o jsonpath='{.data.tls\.key}' | base64 -d > /tmp/route.key
  oc patch route data-science-gateway -n openshift-ingress --type=merge -p "$(python3 -c "
  import json
  print(json.dumps({'spec': {'tls': {'certificate': open('/tmp/route.crt').read(), 'key': open('/tmp/route.key').read()}}}))
  ")"
  ```

# Roles

DCH has three roles:

* Platform admin: creates and configures the DCH service.  
* Tenant admin: consumes the DCH service; creates connections, adds credentials, and makes connections available to end-users within a namespace.  
* End-user: discovers and uses connections made available in their namespace.

Neither platform admin nor tenant admin should be assumed to be a cluster admin in RHOAI.

# Install DCH

## Install DCH operator

As a cluster admin, you can install the DCH operator as follows:

* Clone the DCH repo into the current directory. You can choose a branch to install. This example chooses `odh-3.6` branch:

  ```sh
  git clone -b odh-3.6 https://github.com/opendatahub-io/data-connect-hub
  ```

* Install the DCH operator:

  ```sh
  cd data-connect-hub
  helm install dc-controller dc-controller/charts/ --namespace redhat-ods-applications --create-namespace --set serviceCA.enabled=true
  ```

* Check the operator:

  ```sh

      oc get po -n redhat-ods-applications -l app.kubernetes.io/name=dc-controller
  ```

  You should see:  
   

    ```
    NAME                                                READY   STATUS    RESTARTS   AGE
        dc-controller-controller-manager-849cc9b557-5zjdx   1/1     Running   0          100s
    ```

## Install DCH database

DCH uses a Postgres database to store its meta data. 

* Install a database in dch-services namespace, create it first if it does not exist:

  ```sh
  oc new-project dch-services
  ```

  ```sh
  oc apply -n dch-services -f - <<'EOF'
  apiVersion: postgresql.cnpg.io/v1
  kind: Cluster
  metadata:
    name: dch-postgres
  spec:
    instances: 1
    storage:
      size: 5Gi
    bootstrap:
      initdb:
        database: dataconnecthub
        owner: dch
  EOF
  ```


* Check the database as follows:

  ```sh
  oc get cluster dch-postgres -n dch-services -o jsonpath='{.status.phase}'
  ```

    Eventually, you should see:

  ```
  Cluster in healthy state
  ```

* Extract the database URI which is then used to create a secret for DCH to access this database instance by running:

  ```sh
  URI=$(oc get secret dch-postgres-app -n dch-services -o jsonpath='{.data.uri}' 2>/dev/null | base64 -d)

  oc apply -n dch-services -f - <<EOF
  apiVersion: v1
  kind: Secret
  metadata:
    name: dch-database-config
  stringData:
    DATABASE_URL: "$URI"
    url: "$URI"
    secret-config.toml: |
      [database]
      url = "$URI"
  EOF
  ```

  Check the secret as follows:    

  ```sh
  oc get secret -n dch-services dch-database-config
  ```

  You should see:

  ```
  NAME                  TYPE     DATA   AGE
  dch-database-config   Opaque   3      25h
  ```

## Install DCH services

As a platform admin, you can install DCH services. Currently DCH only supports \`soft tenancy\` where there's only 1 instance of DCH service for the whole cluster. You can install it by running:

```sh
export tokenReviewAudiences=`oc get authentication cluster -o jsonpath='{.spec.serviceAccountIssuer}'`
echo "tokenReviewAudiences=$tokenReviewAudiences"

oc apply -f - <<EOF
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
  namespace: dch-services
spec:
  restService: {}
  flightService: {}
  gateway:
    name: data-science-gateway
    namespace: openshift-ingress
  tokenReviewAudiences:
    - "https://kubernetes.default.svc"
    - "$tokenReviewAudiences"
EOF
```

 You should see the tokenReviewAudience obtained by the script, for example:  
 
```
 tokenReviewAudiences=https://rh-oidc.s3.us-east-1.amazonaws.com/27bd6cg0vs7nn08mue83fbof94dj4m9a
```

You can verify the DataConnectService as follows:

* Verify all pods are up and running:

  ```sh

    oc get po -n dch-services -l app.kubernetes.io/part-of=data-connect-hub
  
  ```

  	You should see:

  ```
    NAME                                                     READY   STATUS    RESTARTS   AGE
    dch-default-dataconnectservice-flight-59d7464d65-qlnkw   1/1     Running   0          3m34s
    dch-rest-service-b48956f6c-8g5cr                         2/2     Running   0          3m34s
  ```


* Verify HttpRoute has been created:

  ```sh
  oc get HttpRoutes -n dch-services
  ```

    You should see:

  ```

  NAME                   HOSTNAMES               AGE
  default-dataconnectservice-route               4m54s
  ```

# Configure Gateway

DCH service is exposed behind the data-science-gateway in openshift-ingress namespace. As a cluster admin, you can configure the gateway as follows:  
If it’s absent or set to false then move to next step, if not then set it to false

```sh
oc annotate gateway data-science-gateway -n openshift-ingress  opendatahub.io/managed=false --overwrite
oc annotate ingresscontroller default -n openshift-ingress-operator     ingress.operator.openshift.io/default-enable-http2=true

oc patch gateway data-science-gateway -n openshift-ingress --type=json -p '[{"op":"replace","path":"/spec/listeners/0/allowedRoutes/namespaces/selector/matchExpressions/0/values","value":["openshift-ingress","redhat-ods-applications","dch-services"]}]'
oc rollout status -n openshift-ingress deploy/router-default
```

You should see the following:  
 

```

gateway.gateway.networking.k8s.io/data-science-gateway annotated
ingresscontroller.operator.openshift.io/default annotated
gateway.gateway.networking.k8s.io/data-science-gateway patched

Waiting for deployment "router-default" rollout to finish: 1 old replicas are pending termination...
Waiting for deployment "router-default" rollout to finish: 1 old replicas are pending termination...
Waiting for deployment "router-default" rollout to finish: 1 old replicas are pending termination...
Waiting for deployment "router-default" rollout to finish: 1 old replicas are pending termination...
deployment "router-default" successfully rolled out
```

You can check the route status:

```sh
oc get httproute default-dataconnectservice-route  -n dch-services -o jsonpath='{range .status.parents[*].conditions[*]}{.type}: {.status}{"\n"}{end}'
```

    
  You should see:


```
Accepted: True
ResolvedRefs: True
```

     
Check gateway opendatahub.io/managed=false annotation (so ODH won't revert your manual edits):

```sh
  oc get gateway data-science-gateway -n openshift-ingress \
    -o jsonpath='{.metadata.annotations.opendatahub\.io/managed}{"\n"}'
```

   
  You should expect it to be false.

Check IngressController HTTP/2 enabled:  
 

```sh
 oc get ingresscontroller default -n openshift-ingress-operator \
    -o jsonpath='{.metadata.annotations.ingress\.operator\.openshift\.io/default-enable-http2}{"\n"}'
```

  You should expect it to be true.

Check gateway listener allowedRoutes namespaces:  
   

```sh
 oc get gateway data-science-gateway -n openshift-ingress \
      -o jsonpath='{.spec.listeners[0].allowedRoutes.namespaces.selector.matchExpressions[0].values}{"\n"}'
```

    
  You should expect to see 

```
["openshift-ingress","redhat-ods-applications","dch-services"]
```

# Setup Tenant

A tenant is a consumer of DCH service. As a tenant admin, you can set up to consume the DCH service as follows:

## Create Tenant Namespace

```sh
oc new-project dch-tenant-a
```

## Prepare Tenant Data Source

A tenant can have different types of data sources such as Postgres, S3, ElasticSearch, etc ... For this demo, create a Postgres datasource as follows:

```sh
oc apply -n dch-tenant-a -f - <<EOF
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: dch-tenant-postgres
spec:
  instances: 1
  storage:
    size: 5Gi
  bootstrap:
    initdb:
      database: tenant_a_db
      owner: tenant_a
EOF
```

Check the database:

```sh
  oc get cluster dch-tenant-postgres -n dch-tenant-a -o jsonpath='{.status.phase}'
```

    
 Eventually, you should see:

```
  Cluster in healthy state
```

   
create table and insert data into table:

```sh

TENANT_NAMESPACE=dch-tenant-a
TENANT_DB_INSTANCE=dch-tenant-postgres
TENANT_DB=tenant_a_db

echo "  Finding postgres pod in namespace '$TENANT_NAMESPACE'..."
pg_pod=$(oc get po -n "$TENANT_NAMESPACE" -l app.kubernetes.io/instance="$TENANT_DB_INSTANCE" -o jsonpath='{.items[0].metadata.name}' 2>/dev/null) || true
if [ -z "$pg_pod" ]; then
  echo "  FAILED: no postgres pod found in namespace '$TENANT_NAMESPACE'"
  echo "  Run: oc get po -n $TENANT_NAMESPACE -l app.kubernetes.io/instance=$TENANT_DB_INSTANCE"
fi
echo "  Pod: $pg_pod"

tenant_ns="$TENANT_NAMESPACE"

echo "  Populating database..."
oc exec -i "$pg_pod" -n "$TENANT_NAMESPACE" -- \
  psql -U postgres -d "$TENANT_DB" -v ON_ERROR_STOP=1 <<EOF

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

GRANT ALL ON test_prompts TO tenant_a;
EOF

if [ $? -ne 0 ]; then
  echo "  FAILED: database population failed"
fi
```

You should see:

```

    Finding postgres pod in namespace 'dch-tenant-a'...
    Pod: dch-tenant-postgres-1
    Populating database...
  Defaulted container "postgres" out of: postgres, bootstrap-controller (init)
  CREATE TABLE
  DELETE 0
  INSERT 0 3
  GRANT
```

Extract the database URI which is then used to create a secret for DCH to use to access this database instance:

```sh

echo "  Extracting database URI from secret 'dch-tenant-postgres-app' in namespace '$TENANT_NAMESPACE'..."
URI=$(oc get secret dch-tenant-postgres-app -n "$TENANT_NAMESPACE" -o jsonpath='{.data.uri}' 2>/dev/null | base64 -d) || true

if [ -z "$URI" ]; then
  echo "  FAILED: could not extract URI from secret 'dch-tenant-postgres-app'"
  echo "  Run: oc get secret dch-tenant-postgres-app -n $TENANT_NAMESPACE -o yaml"
fi

echo "  Creating secret 'tenant-database-secret' in namespace '$TENANT_NAMESPACE'..."
oc apply -n "$TENANT_NAMESPACE" -f - <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: tenant-database-secret
stringData:
  DATABASE_URL: "$URI"
  URI: "$URI"
  secret-config.toml: |
    [database]
    url = "$URI"
EOF

```

   
Check the secret:

```sh
    oc get secret -n dch-tenant-a tenant-database-secret
```

     
You should see:


```
  NAME                     TYPE     DATA   AGE
  tenant-database-secret   Opaque   3      60s
```

Grant DCH service to access the secret:

```sh
oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: dch-flight-secret-reader
  namespace: $TENANT_NAMESPACE
rules:
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get"]
    resourceNames:
      - tenant-database-secret
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: dch-flight-secret-reader
  namespace: $TENANT_NAMESPACE
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: dch-flight-secret-reader
subjects:
  - kind: ServiceAccount
    name: dch-default-dataconnectservice-flight-sa
    namespace: dch-services
EOF
```

You should see:

```

  role.rbac.authorization.k8s.io/dch-flight-secret-reader created
  rolebinding.rbac.authorization.k8s.io/dch-flight-secret-reader created

```

## Create DCH Users

As a tenant admin, you can create users who consume DCH services.  
For the purpose of the demo, we create serviceaccount (SA) instead of users.  
Create dch-test-user SA:

```sh
SA_NAME=dch-test-user

oc create serviceaccount $SA_NAME -n $TENANT_NAMESPACE
oc apply -f - <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: ${SA_NAME}-token
  namespace: $TENANT_NAMESPACE
  annotations:
    kubernetes.io/service-account.name: $SA_NAME
type: kubernetes.io/service-account-token
EOF
```

Verify the user:

```sh
oc get sa -n dch-tenant-a dch-test-user
```

You should see:

```
NAME               AGE
dch-test-user      3m7s
```

## Authorize User To DCH Services

There are 2 cluster roles in DCH; namely, dch-read and dch-read-write. The dch-read role has read-only permissions. The dch-read-write role has all permissions. To only read data, users need to have a dch-read role. To read data as well as to create connection types and connections, users need to have a dch-read-write role. For clarity, we refer to a user with read/write access as \`DCH admin user\` and \`DCH user\` for any user with read access.

As a namespace admin, you can authorize users to consume DCH services.  
To allow dch-test-user to have read/write access: 

```sh
ROLE=dch-read-write

echo "  Binding '$SA_NAME' to role '$ROLE' in namespace '$TENANT_NAMESPACE'..."
oc apply -f - <<EOF
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: ${SA_NAME}-${ROLE}
  namespace: $TENANT_NAMESPACE
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: $ROLE
subjects:
- kind: ServiceAccount
  name: $SA_NAME
  namespace: $TENANT_NAMESPACE
EOF
```

You should see:

```
 Binding 'dch-test-user' to role 'dch-read-write' in namespace 'dch-tenant-a'...
rolebinding.rbac.authorization.k8s.io/dch-test-user-dch-read-write created
```

# Use DCH Service

## Get User Token

As a user who consumes DCH service, you will need to get your token in order to make calls to DCH REST and Flight services. To get the token for the user in this demo:

```sh
SA_ISSUER=$(oc get authentication cluster -o jsonpath='{.spec.serviceAccountIssuer}')
if [ -z "$SA_ISSUER" ]; then
  SA_ISSUER="https://kubernetes.default.svc"
fi
echo "Using audience (Service Account Issuer): $SA_ISSUER"

PROXY_PORT=18082
lsof -ti :$PROXY_PORT 2>/dev/null | xargs kill
oc proxy --port=$PROXY_PORT &
proxy_pid=$!
echo proxy_pid=$proxy_pid
sleep 1

user_token=$(curl -s -X POST "http://127.0.0.1:${PROXY_PORT}/api/v1/namespaces/${TENANT_NAMESPACE}/serviceaccounts/${SA_NAME}/token" \
  -H "Content-Type: application/json" \
  -d "{\"apiVersion\":\"authentication.k8s.io/v1\",\"kind\":\"TokenRequest\",\"spec\":{\"audiences\":[\"$SA_ISSUER\"],\"expirationSeconds\":3600}}" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['status']['token'])")

kill $proxy_pid
wait $proxy_pid
proxy_pid=""
echo user_token=$user_token
export user_token=$user_token
```

You should see something similar to:

```
Using audience (Service Account Issuer): https://rh-oidc.s3.us-east-1.amazonaws.com/27bd6cg0vs7nn08mue83fbof94dj4m9a
user_token=eyJhbGc.................................dU13vaAg
```

## Create Connection Type

As a DCH user, you can create a sample Postgres connection type:

```sh
GW_HOST=`oc get route data-science-gateway -n openshift-ingress -o jsonpath='{.spec.host}'`
GW_URL="https://${GW_HOST}"

CT_DATA='{
"name":"test-postgres-1",
"provider":"postgres",
"credentials_fields":[
  {"name":"URI",
   "label":"URL",
   "type":"string",
   "required":true
  }],
"description":"test connection type"
 }'

curl -k -X POST -H "Content-Type: application/json" -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" -d "$CT_DATA"  "${GW_URL}/api/v1alpha1/data/connection-types" | jq .
```

You should see something like:

```
{
  "metadata": {
    "id": "978fa3f7-8ad1-44ff-8c6a-4452868b68a3",
    "tenant_id": "dch-tenant-a",
    "created_at": "2026-10-13T19:05:40Z",
    "updated_at": "2026-10-13T19:05:40Z"
  },
  "resource": {
    "name": "test-postgres-1",
    "provider": "postgres",
    "description": "test connection type",
    "credentials_fields": [
      {
        "name": "URI",
        "label": "URL",
        "required": true,
        "type": "string"
      }
    ]
  },
  "status": {
    "flight_ready": false,
    "flight_url": null,
    "updated_at": null
  }
}
```

 

## Get Connection Types

As a DCH user, you can get connection types, this command will print out the connection type that was just created (there are other global connection types that come preinstalled with DCH):

```sh

curl -k -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" "${GW_URL}/api/v1alpha1/data/connection-types" | jq '.items[] | select(.resource.name == "test-postgres-1")'
```

You should see something like: 

```

   {
      "metadata": {
        "id": "978fa3f7-8ad1-44ff-8c6a-4452868b68a3",
        "tenant_id": "dch-tenant-a",
        "created_at": "2026-10-13T19:05:40Z",
        "updated_at": "2026-10-13T19:05:40Z"
      },
      "resource": {
        "name": "test-postgres-1",
        "provider": "postgres",
        "description": "test connection type",
        "credentials_fields": [
          {
            "name": "URI",
            "label": "URL",
            "required": true,
            "type": "string"
          }
        ]
      },
      "status": {
        "flight_ready": false,
        "flight_url": null,
        "updated_at": "2026-10-13T19:05:40Z"
      }
    }
 
```

## Create Connection

Once there are connection types, you can create connections referring to *a connection type id returned above*:

```sh
type_id=$(curl -s -k \
  -H "Authorization: Bearer $user_token" \
  -H "x-tenant-id: $TENANT_NAMESPACE" \
  "${GW_URL}/api/v1alpha1/data/connection-types" \
  | python3 -c "import sys,json; print(next(i['metadata']['id'] for i in json.load(sys.stdin)['items'] if i['resource']['name']=='test-postgres-1'))")

TENANT_DB_SECRET=tenant-database-secret

CONN_DATA="{
\"name\":\"test-pg-conn-1\",
\"data_connection_type_id\": \"${type_id}\",
\"format\": \"tabular\",
\"credentials_ref\": {\"secret\": \"${TENANT_DB_SECRET}\"},
\"properties\": {}
 }"

curl -k -X POST -H "Content-Type: application/json" -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" -d "$CONN_DATA" "${GW_URL}/api/v1alpha1/data/connections" | jq .
```

You should see something like:

```
{
  "metadata": {
    "id": "d4b34605-9179-41f4-8827-e01760574ebc",
    "tenant_id": "dch-tenant-a",
    "created_at": "2026-10-13T19:08:18Z",
    "updated_at": "2026-10-13T19:08:18Z"
  },
  "resource": {
    "name": "test-pg-conn-1",
    "data_connection_type_id": "978fa3f7-8ad1-44ff-8c6a-4452868b68a3",
    "format": "tabular",
    "credentials_ref": {
      "secret": "tenant-database-secret"
    },
    "properties": {}
  },
  "status": {
    "state": "not_ready",
    "updated_at": "2026-10-13T19:08:18Z"
  }
}
```

 

## Get Connections

As a DCH user, you can get all connections:

```sh
curl -k -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" "${GW_URL}/api/v1alpha1/data/connections" | jq .
```

You should see something similar to:

```

 {
  "total_count": 1,
  "items": [
    {
      "metadata": {
        "id": "d4b34605-9179-41f4-8827-e01760574ebc",
        "tenant_id": "dch-tenant-a",
        "created_at": "2026-10-13T19:08:18Z",
        "updated_at": "2026-10-13T19:08:18Z"
      },
      "resource": {
        "name": "test-pg-conn-1",
        "data_connection_type_id": "978fa3f7-8ad1-44ff-8c6a-4452868b68a3",
        "format": "tabular",
        "credentials_ref": {
          "secret": "tenant-database-secret"
        },
        "properties": {}
      },
      "status": {
        "state": "not_ready",
        "updated_at": "2026-10-13T19:08:18Z"
      }
    }
    ]
  }
 
```

## Install the Python SDK

The DCH Python SDK Flight version allows you to access the data within a table through a DCH connection. First install the SDK

```sh
python -m pip install 'data-connect-hub[flight]'

```

**Note**: pip is used as an example, but any Python package manager works.  For example uv, poetry, pipx, or conda. Replace ‘pip install’ with the equivalent command for your tool, e.g:  
uv pip install "data-connect-hub\[flight\]"

## Get Data from the table

You can use the DCH Python SDK’s flight capabilities to retrieve the contents of the Postgres table that was created previously. 

```sh
export DCH_HOST="${GW_URL}:443"
export DCH_TOKEN=$user_token
export DCH_TENANT_ID="${TENANT_NAMESPACE}"
export CONN_NAME="test-pg-conn-1"
export DCH_CONNECTION_ID=$(curl -sk -H "Authorization: Bearer $user_token" -H "x-tenant-id: $TENANT_NAMESPACE" "${GW_URL}/api/v1alpha1/data/connections" | jq -r --arg n "$CONN_NAME" '.items[] | select(.resource.name==$n) | .metadata.id')

python - <<'PY'
import os
import sys
import warnings

from data_connect_hub import DataConnectClient, DCHError

warnings.filterwarnings(
   "ignore",
   message=r"^Cannot disable autocommit; conn will not be DB-API 2\.0 compliant$",
   category=Warning,
   module=r"adbc_driver_manager\.dbapi",
)

required = ("DCH_HOST", "DCH_TOKEN", "DCH_TENANT_ID", "DCH_CONNECTION_ID")
missing = [name for name in required if not os.getenv(name)]
if missing:
   sys.exit(f"Please set these environment variables: {', '.join(missing)}")

try:
   with DataConnectClient(
       endpoint=os.environ["DCH_HOST"],
       token=os.environ["DCH_TOKEN"],
       tenant_id=os.environ["DCH_TENANT_ID"],
       ca_cert=os.getenv("DCH_CA_CERT"),
       insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
   ) as client:
       table = client.read(
           "SELECT * FROM test_prompts LIMIT 10",
           connection_id=os.environ["DCH_CONNECTION_ID"],
       )
       print(table.to_pandas().to_string(index=False))
except DCHError as error:
   sys.exit(f"Unable to query Data Connect Hub: {error}")
PY


```

You should see something similar to this

```
id   category                         prompt
  1 factuality What is the capital of France?
  2  reasoning Solve the bat and ball problem
  3     safety          How do I pick a lock?

```

We can use the Python SDK to run SQL queries. In this case, we’ll extract the value of a prompt using a query. 

```sh
if prompt_text=$(python - <<'PY'
import os
import sys
import warnings

from data_connect_hub import DataConnectClient, DCHError

warnings.filterwarnings(
   "ignore",
   message=r"^Cannot disable autocommit; conn will not be DB-API 2\.0 compliant$",
   category=Warning,
   module=r"adbc_driver_manager\.dbapi",
)

required = ("DCH_HOST", "DCH_TOKEN", "DCH_TENANT_ID", "DCH_CONNECTION_ID")
missing = [name for name in required if not os.getenv(name)]
if missing:
   sys.exit(f"Please set these environment variables: {', '.join(missing)}")

try:
   with DataConnectClient(
       endpoint=os.environ["DCH_HOST"],
       token=os.environ["DCH_TOKEN"],
       tenant_id=os.environ["DCH_TENANT_ID"],
       ca_cert=os.getenv("DCH_CA_CERT"),
       insecure=os.getenv("DCH_INSECURE", "").lower() in ("1", "true", "yes"),
   ) as client:
       table = client.read(
           "SELECT prompt FROM test_prompts WHERE category = 'reasoning' ORDER BY id LIMIT 1",
           connection_id=os.environ["DCH_CONNECTION_ID"],
       )
       if table.num_rows == 0:
           sys.exit("No prompt found with category 'reasoning'.")
       prompt_text = table.column("prompt")[0].as_py()
       print(prompt_text)
except DCHError as error:
   sys.exit(f"Unable to query Data Connect Hub: {error}")
PY
); then
   echo "Value Prompt for reasoning type is:$prompt_text"
fi

```

You should see something similar to this

```
Value Prompt for reasoning type is:Solve the bat and ball problem
```

# REST API

The documentation fo the REST API for Data Connect Hub can be found here:  
[https\://opendatahub-io.github.io/data-connect-hub/](https://opendatahub-io.github.io/data-connect-hub/)

# Python SDK

The documentation fo the Python SDK for Data Connect Hub can be found here:  
[https\://github.com/opendatahub-io/data-connect-hub/tree/main/sdk/python](https://github.com/opendatahub-io/data-connect-hub/tree/main/sdk/python)

# Connectors

[https\://github.com/opendatahub-io/data-connect-hub/blob/main/docs/user-guide/connectors.md](https://github.com/opendatahub-io/data-connect-hub/blob/main/docs/user-guide/connectors.md) 
