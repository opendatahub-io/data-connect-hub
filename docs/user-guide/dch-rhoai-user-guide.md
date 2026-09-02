# Data Connect Hub (DCH) - User Guide for RHOAI OpenShift
The purpose of this document is to provide **end-users** steps to install, configure, use DCH in an **OpenShift** cluster with RHOAI, as such this document can be used by doc team to build official RedHat doc. This approach is similar to other services.

## Content
- [x] Prerequisites
  - [x] CLI tools
  - [x] Namespaces
  - [x] Gateway
    - [ ] Patching Gateway [TBD]
  - [x] Postgres Db
- [x] Install DCH Operator
- [x] Install `DataConnectService`
- [x] Prepare Test Users
  - [x] Create Test User
  - [x] Authorize Test User
  - [x] Get User Token
- [x] Create Connection Type
- [x] Get Connection Types
- [x] Create DB Secret for Connection
- [x] Grant DCH Services to read secrets in tenant namespace 
- [x] Create Connection
- [x] Get Connections
- [x] Populate Test Data 
- [x] Get Data
- [ ] Auto Migrate Existing RHAI Connections and Connection Types
- [ ] S3 Connection
- [x] REST API
- [x] Python SDK
- [x] Trouble Shooting

## Prerequisites
- You have an OpenShift cluster on version `4.20` or higher.
- You have installed the OpenShift CLI (`oc`).
- You have installed `helm` which will be used to install DCH operator.
- You have installed `curl`, `grpcurl`. We will use these to test DCH REST and flight service. There are different versions of `grpcurl` and they work differently. `grpcurl` used in this document was installed with `go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest`.
- You have installed `jq` which will be used to display json.
- You have logged in as a user with cluster-admin privileges - the cluster admin.
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
  - Tenant-infra-namespaces: A tenant's DCH services run in its infrastructure namespace. For Dev Preview, DCH supports only `soft tenancy` model where there is only 1 DCH instance in the whole cluster for all the tenants. For this demo, we will use `dch-infra-example` namespace. You can create a namespace as follows:
    ```
    $ oc new-project dch-infra-example
    ```
  - Tenant-namespaces: A tenant is a user of DCH service. A tenant's credential secrets are in its namespace. For this demo, we will use `dch-example` namespace. You can create a namespace as follows:
    ```
    $ oc new-project dch-example
    ```

- **NB**: For the rest of the document, you need to be in the `scripts` folder in order to run the scripts in this folder.
- A `Gateway` which will be referred to by `DataConnectService` CR. You can use an existing `Gateway`. For the purpose of this demo, we will create a `Gateway` called `dch-gateway` in `openshift-ingress` namespace by running the [scripts/create-gateway.sh](scripts/create-gateway.sh). You can check the gateway as follows:
  ```console
  oc get gateway -n openshift-ingress dch-gateway
  NAME          CLASS                        ADDRESS                                                                      PROGRAMMED   AGE
  dch-gateway   data-science-gateway-class   dch-gateway-data-science-gateway-class.openshift-ingress.svc.cluster.local   True         97s
  ```
   - Eventually, each tenant can have its own gateway.

- A Postgres database to store DCH meta data. For this demo, we also use this database to store data. You can prepare Postgres database as follows:
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

### Install `DataConnectService`
Currently DCH only support `soft tenancy` model where there's only 1 instance of DCH service in the whole cluster. All tenants share this one DCH service, and this instance will be managed by cluster admin, not by tenant admins.


Once the DCH operator is running and there's an available `Gateway`, as a cluster admin, you can install the `DataConnectService`. This creates a REST service, a flight service, and `HttpRoute` attaching to the `Gateway`. Run the commands in [scripts/install-dch-services.sh](scripts/install-dch-services.sh) to create `DataConnectService` CR in `dch-infra-example` namespace.

You can verify the `DataConnectService` as follows:
- Verify all pods are up and running:
  ```
  $ oc get po -n dch-infra-example -l app.kubernetes.io/part-of=data-connect-hub
  NAME                              READY   STATUS    RESTARTS   AGE
  flight-service-7475479d7-6gq7w   1/1     Running   0          23h
  rest-service-5987596fcf-4r4b8    2/2     Running   0          27h
  ```
- Verify HttpRoute has been created:
  ```console
  $ oc get HttpRoutes -n dch-infra-example
  NAME                   HOSTNAMES   AGE
  dch-data-connect-hub               120m
  ```

### Prepare Test Users
There are 2 cluster roles in DCH; namely, `dch-read` and `dch-read-write`. The `dch-read` role has read-only permissions. The `dch-read-write` role has all permissions. To only ingest data, users need to have `dch-read` role. To ingest data as well as to create connection types and connections, users need to have `dch-read-write` role. For clarity, we refer to a user with read/write access as `DCH admin user` and `DCH user` for any user with read access.

#### Create Test Users
The cluster admin can create users who consume DCH services.
For the purpose of the demo, we create `serviceaccount` (SA) instead of users in `dch-example` tenant namespace.
You can run the commands in [scripts/create-test-user.sh](scripts/create-test-user.sh) to create `dch-test-user` SA in `dch-example` namespace:

You can verify the users as follows:
```console
$ oc get sa -n dch-example dch-test-user
NAME               SECRETS   AGE
dch-test-user      1         3m7s
```

#### Authorize Test User
The cluster admin can authorize users to consume the tenant's DCH services.
To allow `dch-test-user` of `dch-example` tenant to have read/write access, you can run the commands in [scripts/auth-test-user.sh](scripts/auth-test-user.sh). You can verify as follows:
```console
$  oc get rolebindings -n dch-example dch-test-user-dch-read-write
NAME                           ROLE                         AGE
dch-test-user-dch-read-write   ClusterRole/dch-read-write   41s
```

#### Get User Token
As a user who consumes DCH services, 
you will need to get your token in order to make calls to REST and flight services. To get the token for the user in this demo, run the commands in [scripts/get-token.sh](scripts/get-token.sh).

### Create Connection Type
As a DCH admin user, you can run the script [scripts/create-connection-type.sh](scripts/create-connection-type.sh) to create a sample Postgres connection type. You should see the following:
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

### Create DB Secret for Connection
A connection refers to a connection type and a database secret in the tenant namespace. So before creating a connection, we need to create a secret for the database in this example. As a tenant admin, run [scripts/create-db-secret.sh](scripts/create-db-secret.sh) to create a secret in `dch-example` namespace. You should see the following:
```console
  Extracting database URI from secret 'dch-postgres-app' in namespace 'dch-infra-example'...
  Creating secret 'dch-database-config' in namespace 'dch-example'...
secret/dch-database-config created
```

### Grant DCH Services to read secrets in tenant namespace 
With the database secrets created in tenant namespaces, we need to grant DCH services in tenant infra namespaces to read secrets from tenant namespaces. As a tenant amin, run [scripts/grant-service-read-secret.sh](scripts/grant-service-read-secret.sh) to grant services from `dch-infra-namespace` to access secrets in `dch-namespace`. You should see the following:
```console
role.rbac.authorization.k8s.io/dch-flight-secret-reader created
rolebinding.rbac.authorization.k8s.io/dch-flight-secret-reader created
```

### Create Connection
Once there are connection types, you can create connections refering to the connection types. As a DCH admin user, you can run the script [scripts/create-connection.sh](scripts/create-connection.sh) to create a connection with the connection type id above. For example:
```console
$ ./create-connection.sh 6a12dc44-7901-4fd2-9d84-c52c12c748b3
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
As a DCH user, you can run the script [scripts/get-connections.sh](scripts/get-connections.sh) to get all connections. You will need the connection `id` to fetch data. The output should be similar to:
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
### Populate Test Data
For the purpose of this demo,
before fetching the data, run the script [scripts/populate-test-data.sh](scripts/populate-test-data.sh) to create table and insert data into table. You should see the following:
```console
================================== POPULATE DB =============================
  Finding postgres pod in namespace 'dch-infra-example'...
  Pod: dch-postgres-1
  Populating database...
Defaulted container "postgres" out of: postgres, bootstrap-controller (init)
CREATE TABLE
INSERT 0 3
GRANT
  Database populated successfully
```

### Get Data
As a DCH user, you can call DCH services to ingest data.
You can run the script [scripts/get-data.sh](scripts/get-data.sh) to get data from a connection using the connection `id`. In addition to getting the data, this script also downloads `grpcurl`, downloads Flight proto file, uses Python `pyarrow` to decode the returned arrow data for display. The output should be similar to:
```console
$ ./get-data.sh 34c998ff-7c28-4c03-a4a9-8a2616513feb
   Finding rest-service pod...
  Pod: dch-rest-service-656c8b84f8-9bqbz
  Port-forwarding dch-rest-service-656c8b84f8-9bqbz:8080 -> localhost:18080...
  Port-forward ready (pid=410279)

  Finding flight-service pod...
  Pod: dch-flight-service-765f8dff9d-87gcn
  Port-forwarding dch-flight-service-765f8dff9d-87gcn:50051 -> localhost:15051...
  Port-forward ready (pid=410306)

Using audience (Service Account Issuer): https://rh-oidc.s3.us-east-1.amazonaws.com/27bd6cg0vs7nn08mue83fbof94dj4m9a
  Token obtained for dch-test-user

  Proto files ready

  SQL: SELECT * FROM test_prompts
  CMD: grpcurl -insecure -H 'Authorization: Bearer <token>' -H 'x-tenant-id: dch-example' -H 'x-data-connection-id: abb27883-8a4d-4ca1-899f-87fd975f9c2c' -d '{"type":"CMD","cmd":"<base64>"}' localhost:15051 arrow.flight.protocol.FlightService/GetFlightInfo
  schema: [344 chars]
  endpoint: [
    {
        "ticket": {
            "ticket": "CkJ0eXBlLmdvb2dsZWFwaXMuY29tL2Fycm93LmZsaWdodC5wcm90b2NvbC5zcWwuVGlja2V0U3RhdGVtZW50UXVlcnkSHAoaU0VMRUNUICogRlJPTSB0ZXN0X3Byb21wdHM="
        }
    }
  totalRecords: -1
  totalBytes: -1
DoGet for SQL query (expect test_prompts data)
  SQL: SELECT * FROM test_prompts
  CMD: grpcurl -insecure -H 'Authorization: Bearer <token>' -H 'x-tenant-id: dch-example' -H 'x-data-connection-id: abb27883-8a4d-4ca1-899f-87fd975f9c2c' -d '{"ticket":"<ticket>"}' localhost:15051 arrow.flight.protocol.FlightService/DoGet
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

### REST API
REST API document can be found [REST API](https://opendatahub-io.github.io/data-connect-hub/)
### Python SDK
Python SDK installation and examples can be found [Python SDK](https://github.com/opendatahub-io/data-connect-hub/tree/main/sdk/python).

## Trouble Shooting
### Message: Connection Refused
- An example of error message:
  ```console
  Failed to connect to dch-gateway-data-science-gateway-class.openshift-ingress.svc port 443: Connection refused
  ```
- Check if gateway pod is running, for example:
  ```console
  $ oc get po -n openshift-ingress | fgrep dch-gateway
  ```
### Message: No healthy upstream
- An example of error message:
  ```console
  no healthy upstream
  ```
- Check if the HttpRoute exists. There must be an HttpRoute connecting to the gateway in the tenant infra namespace. Although, this Httproute is automatically recreated by the DCH operator:
  ```console
  $ oc get httproute -n dch-infra-example
  No resources found in dch-infra-example namespace.
  ```

### Message: [invalid bearer token, token audiences ["..."] is invalid for the target audiences ["..."]]
This happens when the service account issuer used in getting token doesn't match with the service account issuer in `dch-flight-service-config` configmap. Here are the steps:
- Get the service account issuer. If it's empty, then it's assumed to be "https://kubernetes.default.svc":
  ```console
  $ oc get authentication cluster -o jsonpath='{.spec.serviceAccountIssuer}
  ```
- Compare to the entry in `dch-flight-service-config` configmap. If there's no entry, then it's assumed to be "https://kubernetes.default.svc":
  ```console
  $ oc get cm dch-flight-service-config -n dch-infra-example -o yaml | fgrep review
    token_review_audiences = ["https://rh-oidc.s3.us-east-1.amazonaws.com/27bd6cg0vs7nn08mue83fbof94dj4m9a"]
  ```

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