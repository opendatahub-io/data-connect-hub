# Data Connect Hub (DCH) - User Guide for RHOAI OpenShift
The purpose of this document is to provide **end-users** steps to install, configure, use DCH in an **OpenShift** cluster, as such this document can be used by doc team to build official doc. This approach is similar to other services.

## Content
- [x] Prerequisites
  - [x] CLI tools
  - [ ] Namespaces
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
- By design, DCH related components are in different namespaces. Here is the list of the namespaces for you to note:
  - `redhat-ods-applications`: This is where DCH operator runs.
  - `openshift-ingress`: This is where the `data-science-gateway-class` `gateways` are.
  - Tenant-infra-namespaces: A tenant's DCH services run in its infrastructure namespace. For this demo, we will use `dch-infra-example` namespace. You can create a namespace as follows:
    ```
    $ oc new-project dch-infra-example
    ```
  - Tenant-namespaces: A tenant's credential secrets are in its namespace. For this demo, we will use `dch-example` namespace. You can create a namespace as follows:
    ```
    $ oc new-project dch-example
    ```

- A `Gateway` which will be referred to by `DataConnectService` CR. You can use an existing `Gateway`. For the purpose of this demo, we will create a `Gateway` called `dch-gateway` in `openshift-ingress` namespace by running the [scripts/create-gateway.sh](scripts/create-gateway.sh). You can check the gateway as follows:
  ```console
  oc get gateway -n openshift-ingress dch-gateway
  NAME          CLASS                        ADDRESS                                                                      PROGRAMMED   AGE
  dch-gateway   data-science-gateway-class   dch-gateway-data-science-gateway-class.openshift-ingress.svc.cluster.local   True         97s
  ```
   - Eventually, each tenant can have its own gateway.

- A Postgres database to store data for this demo. DCH requires a Postgres database to store its meta data:
  - First, run the script [scripts/install-postgres-operator.sh](scripts/install-postgres-operator.sh) to install the Postgres operator. You can check the operator as follows:
    ```console
    $ oc get csv -n openshift-operators -l operators.coreos.com/cloudnative-pg.openshift-operators=
    NAME                     DISPLAY         VERSION   REPLACES                 PHASE
    cloudnative-pg.v1.30.0   CloudNativePG   1.30.0    cloudnative-pg.v1.29.2   Succeeded
    ``` 
  - Next, run the script [scripts/create-postgres-db.sh](scripts/create-postgres-db.sh) to install the database in `dch-infra-example` namespace. You can check the database as follows:
    ```console
    $ oc get cluster dch-postgres -n dch-infra-example -o jsonpath='{.status.phase}'
    Cluster in healthy state
    ```
  - Next, run the script [scripts/create-postgres-secret.sh](scripts/create-postgres-secret.sh) to extract the database URI which is then used to create a secret for DCH to use to access this database instance. You can check the secret as follows:
    ```console
    $ oc get secret -n dch-infra-example dch-database-config
    NAME                  TYPE     DATA   AGE
    dch-database-config   Opaque   3      25h
    ```

## Install DCH Operator
### Install with `Helm`
As a cluster admin, you can install DCH operator.
- For Dev Preview (DP), you can install the operator as follows:
  - Clone the repo `https://github.com/red-hat-data-services/data-connect-hub`
  - Change directory to `data-connect-hub`.
  - Run the commands in [scripts/install-operator.sh](scripts/install-operator.sh). This installs the operator in `redhat-ods-applications` namespace. You can check the DCH operator as follows:
    ```console
    $ oc get po -n redhat-ods-applications -l app.kubernetes.io/name=dc-controller
    NAME                                                READY   STATUS    RESTARTS   AGE
    dc-controller-controller-manager-849cc9b557-5zjdx   1/1     Running   0          100s
    ```
- For Post Dev Preview, you can install the operator as follows: [TBD]
### Grant Service Access to Tenant Secrets
[TODO: THIS NEEDS TO BE REWORKED!]
For **each** tenant, the admin must **explicitly** grant permission for DCH services in the tenant infra namespace to read **connection secrets** in the tenant namespace so that the services can access the connections. 

In this demo, it's the `dch-database-config` above. You can run the commands in [scripts/grant-service-read-secret.sh](scripts/grant-service-read-secret.sh). The output should be as follows:
```console
role.rbac.authorization.k8s.io/dch-flight-secret-reader created
rolebinding.rbac.authorization.k8s.io/dch-flight-secret-reader created
```

### Install `DataConnectService`
As a tenant admin, you can install `DataConnectService` into your tenant namespace - once the DCH operator is running and there's an available `Gateway`. This will create a REST service, a flight service, and `HttpRoute` attaching to the `Gateway`. 
- For TP, you can create a `DataConnectService` in `dch-infra-example` namespace for this demo as follows:
  ```bash
  oc apply -f - <<'EOF'
  apiVersion: dataconnecthub.opendatahub.io/v1alpha1
    kind: DataConnectService
    metadata:
      name: dch-example
      namespace: dch-infra-example
    spec:
      description: "Data Connect Hub for the ML platform"
      restApiReplicas: 2
      flightApiReplicas: 3
      gateway:
        name: dch-gateway
        namespace: openshift-ingress
  EOF
  ```
- For post TP, you can create as follows: [TBD]
### Verify `DataConnectService`
You can verify the `DataConnectService` as follows:
- Verify all pods are up and running:
  ```
  $ oc get po -n dch-infra-example -l app.kubernetes.io/part-of=data-connect-hub
  NAME                              READY   STATUS    RESTARTS   AGE
  flight-service-7475479d7-6gq7w   1/1     Running   0          23h
  rest-service-5987596fcf-4r4b8    2/2     Running   0          27h
  ```

### Prepare Test Users
There are 2 cluster roles in DCH; namely, `dch-read` and `dch-read-write`. The `dch-read` role has read-only permissions. The `dch-read-write` role has all permissions. To only ingest data, users need to have `dch-read` role. To ingest data as well as to create connection types and connections, users need to have `dch-read-write` role.

#### Create Test Users
A tenant admin can create users who consume DCH services.
For the purpose of the demo, we create `serviceaccount` (SA) instead of users in `dch-example` namespace.
You can run the commands in [scripts/create-test-user.sh](scripts/create-test-user.sh) to create `dch-test-user` SA in `dch-example` namespace:

You can verify the users as follows:
```console
$ oc get sa -n dch-example dch-test-user
NAME               SECRETS   AGE
dch-test-user      1         3m7s
```

#### Authorize Test User
A tenant admin can authorize users to consume DCH services.
To allow `dch-test-user` to have `dch-read-write` role, you can run the commands in [scripts/auth-test-user.sh](scripts/auth-test-user.sh). You can verify as follows:
```console
$  oc get rolebindings -n dch-example dch-test-user-dch-read-write
NAME                           ROLE                         AGE
dch-test-user-dch-read-write   ClusterRole/dch-read-write   41s
```

#### Get User Token
As a user who consumes DCH services, 
you will need to get your token in order to make calls to REST and flight services. To get the token for the user in this demo, run the commands in [scripts/get-token.sh](scripts/get-token.sh).

### Create Connection Type
As a DCH user, you can run the script [scripts/create-connection-type.sh](scripts/create-connection-type.sh) to create a sample Postgres connection type. You should see the following:
```console
   Finding rest-service pod...
  Pod: dch-rest-service-798989c455-jl9g6
  Port-forwarding dch-rest-service-798989c455-jl9g6:8080 -> localhost:18080...
  Port-forward ready (pid=78068)

  CMD: curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: dch-example' -d "{
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
100   530  100   337  100   193   1845   1057 --:--:-- --:--:-- --:--:--  2912
{
  "metadata": {
    "id": "6a12dc44-7901-4fd2-9d84-c52c12c748b3",
    "tenant_id": "dch-example",
    "created_at": "2026-08-17T16:44:44Z",
    "updated_at": "2026-08-17T16:44:44Z"
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
Notes:
- The DCH services are running in `dch-infra-example` namespace.
- The tenant user is in `dch-example` namespace, thus `tenant_id` is `dch-example`.

### Get Connection Types
As a DCH user, you can get connection types. In this step, instead of going directly to the REST service, we will
make REST calls from the Gateway which requires user to pass in the obtained token. You can run the script [scripts/get-connection-types.sh](scripts/get-connection-types.sh) to see how an example works. The output should be similar to:
 ```console
    Creating test runner pod...
  Waiting for test runner pod...
pod/dch-test-runner condition met
  Using audience: https://rh-oidc.s3.us-east-1.amazonaws.com/27bd6cg0vs7nn08mue83fbof94dj4m9a
eyJhbGciOiJSUzI1NiIsImtpZCI6InA2NmxtWG5xbEtIaGMycW4xS2YteHlQY18zOG9CNUhPd1RyTjl3eGpCSj...lmY
  CMD: curl -sk -H 'Authorization: Bearer <token>' -H 'x-tenant-id: dch-example' https://dch-gateway-data-science-gateway-class.openshift-ingress.svc/api/v1/data/connection-types
{
  "total_count": 1,
  "items": [
    {
      "metadata": {
        "id": "6a12dc44-7901-4fd2-9d84-c52c12c748b3",
        "tenant_id": "dch-example",
        "created_at": "2026-08-17T16:44:44Z",
        "updated_at": "2026-08-17T16:44:44Z"
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
  ]
}
  ```
- To simplify, for the rest of the document, when possible, we will **directly** hit the REST/flight services instead of the gateway.

### Create Connection
As a DCH user, once there are connection types, you can create connections. For this demo, you can run the script [scripts/create-connection.sh](scripts/create-connection.sh) with the connection type id above. For example:
```console
$ ./create-connection.sh dch-infra-example dch-example 6a12dc44-7901-4fd2-9d84-c52c12c748b3
  Finding rest-service pod...
  Pod: dch-rest-service-d5d5768b-qh4vx
  Port-forwarding dch-rest-service-d5d5768b-qh4vx:8080 -> localhost:18080...
  Port-forward ready (pid=102585)

  CMD: curl -X POST -H 'Content-Type: application/json' -H 'x-tenant-id: dch-example' -d "{
"name":"test-pg-conn",
"data_connection_type_id": "6a12dc44-7901-4fd2-9d84-c52c12c748b3",
"format": "tabular",
"admin": {"secret_ref": "dch-database-config"},
"properties": {}
 }" http://localhost:18080/api/v1/data/connections
  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current
                                 Dload  Upload   Total   Spent    Left  Speed
100   574  100   394  100   180   2934   1340 --:--:-- --:--:-- --:--:--  4251
{
  "metadata": {
    "id": "34c998ff-7c28-4c03-a4a9-8a2616513feb",
    "tenant_id": "dch-example",
    "created_at": "2026-08-17T18:17:02Z",
    "updated_at": "2026-08-17T18:17:02Z"
  },
  "resource": {
    "name": "test-pg-conn",
    "data_connection_type_id": "6a12dc44-7901-4fd2-9d84-c52c12c748b3",
    "format": "tabular",
    "admin": {
      "secret_ref": "dch-database-config"
    },
    "properties": {}
  },
  "status": {
    "state": "not_ready",
    "message": null,
    "phases": []
  }
}
```

### Get Connections
You can run the script [scripts/get-connections.sh](scripts/get-connections.sh) to get all connections. These connections reference the connection types above. The output should be similar to:
  ```console
     Finding rest-service pod...
  Pod: dch-rest-service-d5d5768b-qh4vx
  Port-forwarding dch-rest-service-d5d5768b-qh4vx:8080 -> localhost:18080...
  Port-forward ready (pid=104747)

  CMD: curl -H 'x-tenant-id: dch-example' http://localhost:18080/api/v1/data/connections
{
  "total_count": 1,
  "items": [
    {
      "metadata": {
        "id": "34c998ff-7c28-4c03-a4a9-8a2616513feb",
        "tenant_id": "dch-example",
        "created_at": "2026-08-17T18:17:02Z",
        "updated_at": "2026-08-17T18:17:02Z"
      },
      "resource": {
        "name": "test-pg-conn",
        "data_connection_type_id": "6a12dc44-7901-4fd2-9d84-c52c12c748b3",
        "format": "tabular",
        "admin": {
          "secret_ref": "dch-database-config"
        },
        "properties": {}
      },
      "status": {
        "state": "not_ready",
        "message": null,
        "phases": []
      }
    }
  ```

### Get Data
As a DCH user, you can call DCH services to ingest data.
You can run the script [scripts/get-data-from-connection-id.sh](scripts/get-data-from-connection-id.sh) to get data from a connection id. In addition to getting the data, this script also downloads `grpcurl`, downloads Flight proto file, uses Python `pyarrow` to decode the returned arrow data for display. The output should be similar to:
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
### Get Gateway Log
Here's an example of getting gateway log:
```console
$ oc get pods -n openshift-ingress -l gateway.networking.k8s.io/gateway-name=dch-gateway
NAME                                                      READY   STATUS    RESTARTS   AGE
dch-gateway-data-science-gateway-class-5cf694778b-5fjlb   1/1     Running   0          4h8m

$ oc logs -n openshift-ingress dch-gateway-data-science-gateway-class-5cf694778b-5fjlb  -f
2026-08-17T12:57:54.551993Z     info    FLAG: --concurrency="0"
2026-08-17T12:57:54.552038Z     info    FLAG: --domain="openshift-ingress.svc.cluster.local"
```
### Get HttpRoute Status