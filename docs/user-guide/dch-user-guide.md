# Data Connect Hub (DCH) - User Guide for RHOAI OpenShift
The purpose of this document is to provide **end-users** steps to install, configure, use DCH in an **OpenShift** cluster, as such this document can be used by doc team to build official doc. This approach is similar to other services.

## Content
- [x] Prerequisites
  - [x] CLI tools
  - [x] Gateway 
  - [x] Postgres Db
- [x] Install DCH Operator
- [ ] Install `DataConnectService`
- [x] Grant Flight Service Access to Tenant Secrets 
- [x] Verify `DataConnectService`
- [x] Prepare Test Users
  - [x] Create Test User
  - [x] Authorize Test User
  - [x] Get User Token
- [ ] populate-db-temp.sh 
- [ ] Create Connection Type
- [x] Get Connection Types
- [ ] Create Connection
- [x] Get Connections
- [x] Get Data
- [ ] Auto Migrate Existing RHAI Connections and Connection Types
- [ ] S3 Connection
  - [ ] Flight Service jsonl files ingestion - S3
- [ ] Python SDK
- [ ] Trouble Shooting

## Prerequisites
- You have an OpenShift cluster on version `4.20` or higher.
- You have installed the OpenShift CLI (`oc`).
- You have installed `helm` which will be used to install DCH operator.
- You have installed `curl`, `grpcurl`. We will use these to test DCH REST and flight service. There are different versions of `grpcurl` and they work differently. `grpcurl` used in this document was installed with `go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest`.
- You have logged in as a user with cluster-admin privileges.
- You have installed {productname-long} {vernum}.
- A `DataScienceClusterInitialization` (DSCI) exists in your cluster. The `DataScienceClusterInitialization` gets created by the Red Hat OpenShift-AI operator out of the box. Verify DSCI as follows:
  ```
  $ oc get dsci -A

  NAME           AGE   PHASE   CREATED AT
  default-dsci   83d   Ready   2026-05-08T12:41:52Z
  ```
- A `Gateway` which will be referred to by `DataConnectService` CR. You can use an existing `Gateway`. For the purpose of this demo, we will create a `Gateway` called `dch-gateway` in `openshift-ingress` namespace by running the [scripts/create-gateway.sh](scripts/create-gateway.sh). You can check the gateway as follows:
  ```console
  oc get gateway -n openshift-ingress dch-gateway
  NAME          CLASS                        ADDRESS                                                                      PROGRAMMED   AGE
  dch-gateway   data-science-gateway-class   dch-gateway-data-science-gateway-class.openshift-ingress.svc.cluster.local   True         97s
  ```
 
- A namespace called `dch-example` for this demo. You can create a namespace as follows:
  ```
  $ oc new-project dch-example
  ```

- A Postgres database to store data for this demo:
  - First, run the script [scripts/install-postgres-operator.sh](scripts/install-postgres-operator.sh) to install the Postgres operator. You can check the operator as follows:
    ```console
    $ oc get csv -n openshift-operators -l operators.coreos.com/cloudnative-pg.openshift-operators=
    NAME                     DISPLAY         VERSION   REPLACES                 PHASE
    cloudnative-pg.v1.30.0   CloudNativePG   1.30.0    cloudnative-pg.v1.29.2   Succeeded
    ``` 
  - Next, run the script [scripts/create-postgres-db.sh](scripts/create-postgres-db.sh) to install the database. You can check the database as follows:
    ```console
    $ oc get cluster dch-postgres -n dch-example -o jsonpath='{.status.phase}'
    Cluster in healthy state
    ```
  - Next, run the script [scripts/create-postgres-secret.sh](scripts/create-postgres-secret.sh) to extract the database URI which is then used to create a secret for DCH to use to access this database instance. You can check the secret as follows:
    ```console
    $ oc get secret -n dch-example dch-database-config
    NAME                  TYPE     DATA   AGE
    dch-database-config   Opaque   3      25h
    ```

## Install DCH Operator
### Install with `Helm`
- For Dev Preview (DP), you can install the operator as follows:
  - Clone the repo `https://github.com/red-hat-data-services/data-connect-hub`
  - Change directory to `data-connect-hub`.
  - Run the commands in [scripts/install-operator.sh](scripts/install-operator.sh). This installs the operator in `redhat-ods-applications` namespace. You can check the DCH operator as follows:
    ```console
    $ oc get po -n redhat-ods-applications -l app.kubernetes.io/name=dc-controller
    NAME                                                READY   STATUS    RESTARTS   AGE
    dc-controller-controller-manager-849cc9b557-5zjdx   1/1     Running   0          100s
    ```
  
### Install with `DataScienceCluster` (DSC)
  - This will be supported in Technical Preview (TP)
### Install `DataConnectService`
Once the DCH operator is running and there's an available `Gateway`, the next step is to create a `DataConnectService` which will create a REST service, a flight service, and `HttpRoute` attaching to the `Gateway`. You can create a `DataConnectService` for this demo as follows:

```bash
oc apply -f - <<'EOF'
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
  kind: DataConnectService
  metadata:
    name: dch-example
    namespace: dch-example
  spec:
    description: "Data Connect Hub for the ML platform"
    restApiReplicas: 2
    flightApiReplicas: 3
    gateway:
      name: dch-gateway
      namespace: openshift-ingress
EOF
```

### Grant Flight Service Access to Tenant Secrets
For **each** tenant namespace, the admin must **explicitly** grant permission for flight service to read **connection secrets** so that flight service can access connections. In this demo, it's the `dch-database-config` above. You can run the commands in [scripts/grant-service-read-secret.sh](scripts/grant-service-read-secret.sh). The output should be as follows:
```console
role.rbac.authorization.k8s.io/dch-flight-secret-reader created
rolebinding.rbac.authorization.k8s.io/dch-flight-secret-reader created
```

### Verify `DataConnectService`
You can verify the `DataConnectService` as follows:
- Verify all pods are up and running:
  ```
  $ oc get po -n dch-example -l app.kubernetes.io/part-of=data-connect-hub
  NAME                              READY   STATUS    RESTARTS   AGE
  flight-service-7475479d7-6gq7w   1/1     Running   0          23h
  rest-service-5987596fcf-4r4b8    2/2     Running   0          27h
  ```

### Prepare Test Users
There are 2 cluster roles in DCH; namely, `dch-read` and `dch-read-write`. The `dch-read` role has read-only permissions. The `dch-read-write` role has all permissions.

#### Create Test Users
For the purpose of the demo, we create `serviceaccount` (SA) instead of users.
You can run the commands in [scripts/create-test-users.sh](scripts/create-test-users.sh) to create two service accounts in `dch-example` namespace:
- `dch-test-user` — authorized user (bound to `dch-read` role)
- `dch-test-noauth` — unauthorized user (no RoleBinding)

You can verify the users as follows:
```console
$ oc get sa -n dch-example dch-test-user dch-test-noauth
NAME               SECRETS   AGE
dch-test-user      1         3m7s
dch-test-noauth    1         3m7s
```

#### Authorize Test User
To allow `dch-test-user` to have `dch-read` role, you can run the commands in [scripts/auth-test-user.sh](scripts/auth-test-user.sh). You can verify as follows:
```console
$ oc get rolebindings -n dch-example dch-test-user-dch-read
NAME                     ROLE                   AGE
dch-test-user-dch-read   ClusterRole/dch-read   10m
```

#### Get User Token
You will need to get user token in order to make calls to REST and flight services. To get the token for the user in this demo, run the commands in [scripts/get-token.sh](scripts/get-token.sh).

### Create Connection Type
You can run the script [scripts/create-connection-types.sh](scripts/create-connection-types.sh) to create a sample Postgres connection type. You should see the following:
```console
  Finding rest-service pod...
  Pod: rest-service-55c64b79f8-9p2bd
  Port-forwarding rest-service-55c64b79f8-9p2bd:8080 -> localhost:18080...
  Port-forward ready (pid=625386)

  CMD: curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: dch-example' -d "{
  "data_connection_type_id": "00000000-0000-0000-0000-000000000099",
  "name":"test-postgres",
  "provider":"postgresql",
  "description":"test connection type",
  "credentials_fields":[
    {"name":"url",
    "label":"URL",
    "type":"string",
    "required":true
    }]
  }" http://localhost:18080/api/v1/data/connection-types
    % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                  Dload  Upload   Total   Spent    Left  Speed
  100   597  100   337  100   260   1844   1422 --:--:-- --:--:-- --:--:--  3280
  {
    "metadata": {
      "id": "04a25dcb-818a-4db7-bdea-b21fb4c1704e",
      "tenant_id": "dch-example",
      "created_at": "2026-08-14T19:24:55Z",
      "updated_at": "2026-08-14T19:24:55Z"
    },
    "resource": {
      "name": "test-postgres",
      "provider": "postgresql",
      "description": "test connection type",
      "credentials_fields": [
        {
          "name": "url",
          "label": "URL",
          "required": true,
          "type": "string"
        }
      ]
    }
  }
  ```

### Get Connection Types
- Making REST/flight calls from Gateway requires user to pass in the obtained token. You can run the script [scripts/get-connection-types.sh](scripts/get-connection-types.sh) to see how an example works. The output should be similar to:
 ```console
   Creating test runner pod...
   Waiting for test runner pod...
   pod/dch-test-runner condition met
   Using audience: https://rh-oidc.s3.us-east-1.amazonaws.com/27bd6cg0vs7nn08mue83fbof94dj4m9a
  eyJhb.......gcYphA0
    CMD: curl -sk -H 'Authorization: Bearer <token>' -H 'x-tenant-id: dch-example' https://dch-gateway-data-science-gateway-class.openshift-ingress.svc/api/v1/data/connection-types
  {
    "total_count": 1,
    "items": [
      {
        "metadata": {
          "id": "00000000-0000-0000-0000-000000000001",
          "tenant_id": "dch-example",
          "created_at": "2026-01-01T00:00:00Z",
          "updated_at": "2026-01-01T00:00:00Z"
        },
        "resource": {
          "name": "test-postgres-00000000-0000-0000-0000-000000000001",
          "provider": "postgres",
          "description": "Test PostgreSQL",
          "credentials_fields": [
            {
              "name": "url",
              "label": "URL",
              "required": true,
              "type": "string"
            }
          ]
        }
      }
    ]
  }
  ```
- To simplify, for the rest of the document, when possible, we will **directly** hit the REST/flight services instead of the gateway.

### Create Connection
### Get Connections
You can run the script [scripts/get-connections.sh](scripts/get-connections.sh) to get all connections. These connections reference the connection types above. The output should be similar to:
  ```console
    Finding rest-service pod...
    Pod: rest-service-55c64b79f8-9p2bd
    Port-forwarding rest-service-55c64b79f8-9p2bd:8080 -> localhost:18080...
    Port-forward ready (pid=566473)

    CMD: curl -H 'x-tenant-id: dch-example' http://localhost:18080/api/v1/data/connections
    {
      "total_count": 1,
      "items": [
        {
          "metadata": {
            "id": "00000000-0000-0000-0000-000000000002",
            "tenant_id": "dch-example",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
          },
          "resource": {
            "name": "test-pg-conn-00000000-0000-0000-0000-000000000002",
            "data_connection_type_id": "00000000-0000-0000-0000-000000000001",
            "format": "tabular",
            "admin": {
              "secret_ref": "dch-database-config"
            },
            "properties": {}
          },
          "status": {
            "state": "ingestion_not_ready",
            "message": null
          }
        }
      ]
    }
  ```

### Get Data
You can run the script [scripts/get-data-from-connection-id.sh](scripts/get-data-from-connection-id.sh.sh) to get data from a connection id. In addition to getting the data, this script also downloads `grpcurl`, downloads Flight proto file, uses Python `pyarrow` to decode the returned arrow data for display. The output should be similar to:
```console
  Finding flight-service pod...
  Pod: flight-service-74df6d8484-zf5qh
  Port-forwarding flight-service-74df6d8484-zf5qh:50051 -> localhost:15051...
  Port-forward ready (pid=590211)

  Using audience: https://rh-oidc.s3.us-east-1.amazonaws.com/27bd6cg0vs7nn08mue83fbof94dj4m9a
  eyJhbGciOiJSU...kgBM
  Token obtained for dch-test-user

  grpcurl ready
  Proto files ready

  SQL: SELECT * FROM test_prompts
  CMD: grpcurl -plaintext -H 'Authorization: Bearer <token>' -H 'x-tenant-id: dch-example' -H 'x-data-connection-id: 00000000-0000-0000-0000-000000000002' -d '{"type":"CMD","cmd":"<base64>"}' localhost:15051 arrow.flight.protocol.FlightService/GetFlightInfo
  schema: [344 chars]
  endpoint: [
    {
        "ticket": {
            "ticket": "CkJ0eXBlLmdvb2dsZWFwaXMuY29tL2Fycm93LmZsaWdodC5wcm90b2NvbC5zcWwuVGlja2V0U3RhdGVtZW50UXVlcnkSHAoaU0VMRUNUICogRlJPTSB0ZXN0X3Byb21wdHM="
        }
    }
  totalRecords: -1
  totalBytes: -1
  PASSED

DoGet for SQL query (expect test_prompts data)
  SQL: SELECT * FROM test_prompts
  CMD: grpcurl -plaintext -H 'Authorization: Bearer <token>' -H 'x-tenant-id: dch-example' -H 'x-data-connection-id: 00000000-0000-0000-0000-000000000002' -d '{"ticket":"<ticket>"}' localhost:15051 arrow.flight.protocol.FlightService/DoGet
  Rows: 3
  id | category   | prompt
  ---+------------+-------------------------------
  1  | factuality | What is the capital of France?
  2  | reasoning  | Solve the bat and ball problem
  3  | safety     | How do I pick a lock?
```

### S3 Connection
The following steps show how to configure and test an S3 connection:
- You will need to have the following S3 information and export them as follows:
  ```console
  export AWS_S3_ENDPOINT=<your-endpoint-here>
  export AWS_DEFAULT_REGION=<your-region-here>
  export AWS_S3_BUCKET=<your-bucket-here>
  export AWS_ACCESS_KEY_ID=<your-access-key-id-here>
  export AWS_SECRET_ACCESS_KEY=<your-secret-access-key-here>
  ```
- Run the script [scripts/create-s3-dch-config-secret.sh](scripts/create-s3-dch-config-secret.sh) to create secret to store the above S3 information for the S3 connection. You should see:
  ```console
  secret/s3-test-creds created
  ```
- Run the script [scripts/grant-service-read-secret.sh](scripts/grant-service-read-secret.sh) to grant DCH services to read the created secret.

- Run the script [scripts/create-s3-connection-type.sh](scripts/create-s3-connection-type.sh) to create S3 connection type. You should see:
  ```console
    Finding rest-service pod...
      Pod: rest-service-55c64b79f8-9p2bd
      Port-forwarding rest-service-55c64b79f8-9p2bd:8080 -> localhost:18080...
      Port-forward ready (pid=631692)

      CMD: curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: dch-example' -d "{
    "name":"test-s3",
    "provider":"s3",
    "description":"test AWS S3 connection type",
    "credentials_fields":[
      {"name": "AWS_S3_BUCKET", "label": "Bucket", "type": "string", "required": true},
      {"name": "AWS_ACCESS_KEY_ID", "label": "Access Key ID", "type": "string", "required": true},
      {"name": "AWS_SECRET_ACCESS_KEY", "label": "Secret Access Key", "type": "string", "required": true}
      ]
    }" http://localhost:18080/api/v1/data/connection-types
      % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                    Dload  Upload   Total   Spent    Left  Speed
    100   913  100   521  100   392   2852   2146 --:--:-- --:--:-- --:--:--  5016
    {
      "metadata": {
        "id": "84e2486b-d50d-499c-94cb-b7bb83394162",
        "tenant_id": "dch-example",
        "created_at": "2026-08-14T19:50:55Z",
        "updated_at": "2026-08-14T19:50:55Z"
      },
      "resource": {
        "name": "test-s3",
        "provider": "s3",
        "description": "test AWS S3 connection type",
        "credentials_fields": [
          {
            "name": "AWS_S3_BUCKET",
            "label": "Bucket",
            "required": true,
            "type": "string"
          },
          {
            "name": "AWS_ACCESS_KEY_ID",
            "label": "Access Key ID",
            "required": true,
            "type": "string"
          },
          {
            "name": "AWS_SECRET_ACCESS_KEY",
            "label": "Secret Access Key",
            "required": true,
            "type": "string"
          }
        ]
      }
    }
  ```
- 
### Verify Python SDK
Python SDK installation and examples can be found [Python SDK](https://github.com/opendatahub-io/data-connect-hub/tree/main/sdk/python).

## Trouble Shooting