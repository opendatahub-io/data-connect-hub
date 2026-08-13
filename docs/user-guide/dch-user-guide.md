# Data Connect Hub (DCH) - User Guide for RHOAI OpenShift
The purpose of this document is to provide **end-users** steps to install, configure, use DCH in an **OpenShift** cluster, as such this document can be used by doc team to build official doc. This approach is similar to other services.

## Content
- [x] Prerequisites
  - [x] CLI tools
  - [x] Gateway 
  - [x] Postgres Db
- [x] Install DCH Operator
- [ ] Install `DataConnectService`
- [x] Verify `DataConnectService`
- [x] Prepare Test Users
  - [x] Create Test User
  - [x] Authorize Test User
  - [x] Get User Token
- [x] Making REST calls from Gateway
- [ ] Create New Connection Type
- [ ] Get Connection Types
- [ ] Create New Connection
- [ ] Get Connection Data
- [ ] Auto Migrate Existing RHAI Connections and Connection Types
- [ ] Verify S3 Data Connection
- [ ] Flight Service jsonl files ingestion - S3
- [ ] DCH Python SDK
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
- A `Gateway` which will be referred to by `DataConnectService` CR. You can use an existing `Gateway`. For the purpose of this demo, we will create a `Gateway` called `dch-gateway` in `openshift-ingress` namespace by running the [scripts/create-gateway.sh](scripts/create-gateway.sh). You can run the script [scripts/verify-gateway.sh](scripts/verify-gateway.sh) to verify the gateway.
 
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
  - Run the [scripts/install-operator.sh](scripts/install-operator.sh). This script installs the operator in `redhat-ods-applications` namespace. You can check the DCH operator as follows:
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
### Verify `DataConnectService`
You can verify the `DataConnectService` as follows:
- Verify all pods are up and running:
  ```
  $ oc get po -n dch-example -l app.kubernetes.io/part-of=data-connect-hub
  NAME                              READY   STATUS    RESTARTS   AGE
  flight-service-7475479d7-6gq7w   1/1     Running   0          23h
  rest-service-5987596fcf-4r4b8    2/2     Running   0          27h
  ```
- Verify the REST service in the pod is responding by running the script [scripts/verify-rest-from-pod.sh](./scripts/verify-rest-from-pod.sh)

- Verify the flight service in the pod is responding by running the script [scripts/verify-flight-from-pod.sh](./scripts/verify-flight-from-pod.sh)

### Prepare Test Users
There are 2 cluster roles in DCH; namely, `dch-read` and `dch-read-write`. The `dch-read` role has read-only permissions. The `dch-read-write` role has all permissions.

#### Create Test Users
For the purpose of the demo, we create `serviceaccount` (SA) instead of users.
You can run the script [scripts/create-test-users.sh](scripts/create-test-users.sh). This script creates two service accounts in `dch-example` namespace:
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
To allow `dch-test-user` to have `dch-read` role, you can run the script the script [scripts/auth-test-user.sh](scripts/auth-test-user.sh).

#### Get User Token
You will need to get user token in order to make calls to REST and flight services. To get the token for the user in this demo, run the script [scripts/get-token.sh](scripts/get-token.sh).

### Making REST/flight calls from Gateway
- Making REST/flight calls from Gateway requires user to pass in the obtained token. You can run the script [scripts/get-connection-types.sh](scripts/get-connection-types.sh) to see how an example works.
- To simplify, for the rest of the document, when possible, we will directly hit the REST/flight services instead of the gateway.

### Create New Connection Type
### Get Connection Types

#### Verify Rest Service Security
You can run the script [scripts/verify-rest-sec.sh](scripts/verify-rest-sec.sh) to verify REST service authentication, authorization, and TLS. You should see the following output:
```console
================================== VERIFY REST AUTH =============================
--- Testing rest-service via gateway for dch-example ---
  Waiting for port 8443 to be free...
  Port-forwarding dch-gateway-data-science-gateway-class:443 -> localhost:8443...
  Waiting for port-forward...
Forwarding from 127.0.0.1:8443 -> 443
Forwarding from [::1]:8443 -> 443
Handling connection for 8443

Test 1: Unauthenticated request (expect 401)...
  CMD: curl -sk https://localhost:8443/api/v1/data/connections
Handling connection for 8443
  RESPONSE STATUS: 401
  RESPONSE BODY: Unauthorized
  PASSED: unauthenticated request rejected (401)

Test 2: Non-matching path (expect 404)...
  CMD: curl -sk -H 'Authorization: Bearer <token>' https://localhost:8443/api/v2/data/connections
Handling connection for 8443
  RESPONSE STATUS: 404
  RESPONSE BODY:
  PASSED: non-matching path — no route matched (404)

Test 3: Bad token (expect 401)...
  CMD: curl -sk -H 'Authorization: Bearer bad-token' https://localhost:8443/api/v1/data/connections
Handling connection for 8443
  RESPONSE STATUS: 401
  RESPONSE BODY: Unauthorized
  PASSED: bad token correctly rejected (401)

Test 4: Authenticated request (expect 200 or 501)...
  User: cluster-admin
  CMD: curl -sk -H 'Authorization: Bearer <token>' -H 'x-tenant-id: dch-example' https://localhost:8443/api/v1/data/connections
Handling connection for 8443
  RESPONSE STATUS: 501
  RESPONSE BODY: {"code":"unimplemented","message":"Unimplemented"}
  PASSED: authenticated request reached rest-service (501)

ALL PASSED: gateway REST auth tests for dch-example
```
- NOTE: In order to authenticate, the script [scripts/auth-test-user.sh](scripts/auth-test-user.sh) must get appropriate token. Pls review how the script get the token for the appropriate audience.

#### Verify Flight Service Authentication
- First make sure authentication is enabled by the flight service. You can check as follows:
  ```console
  $ oc logs deployment/flight-service -n dch-example | fgrep "Auth enabled"
  2026-08-10T20:40:18.584655Z  INFO flight_service: services/flight/src/main.rs:112: Auth enabled (cache TTL: 300s)
  ```
You can run the script [scripts/verify-flight-sec.sh](scripts/verify-flight-sec.sh) to verify flight service authentication, authorization, and TLS. You should see the following output:
```console
TBF
```

### DCH REST Service
- REST Swagger
  - TODO
- Get the gateway service:
    ```
    $ oc get service -n openshift-ingress dch-gateway-data-science-gateway-class
    NAME                                              TYPE        CLUSTER-IP    EXTERNAL-IP   PORT(S)             AGE
    dch-gateway-data-science-gateway-class   ClusterIP   172.30.67.2   <none>        15021/TCP,443/TCP   88d
    ```
  - Forward gateway service to local:
    ```
    $ oc port-forward svc/dch-gateway-data-science-gateway-class -n openshift-ingress 8443:443 &
    ```
  - Test using `curl`:
    ```
    $ user_token=$(oc whoami -t 2>/dev/null)
    $ curl -sk -H "Authorization: Bearer $user_token" -H 'x-tenant-id: dch-example' https://localhost:8443/api/v1/data/connections

  - You should see:
    ```
    {"code":"unimplemented","message":"Unimplemented"}
    ```
### DCH Flight Service
 - TODO: add header ask
 - Get the gateway service:
    ```
    $ oc get service -n openshift-ingress dch-gateway-data-science-gateway-class
    NAME                                              TYPE        CLUSTER-IP    EXTERNAL-IP   PORT(S)             AGE
    dch-gateway-data-science-gateway-class   ClusterIP   172.30.67.2   <none>        15021/TCP,443/TCP   88d
    ```
  - Forward gateway service to local:
    ```
    $ oc port-forward svc/dch-gateway-data-science-gateway-class -n openshift-ingress 9443:443 &
    ```
  - Test using `grpcurl`:
    ```
    $ user_token=$(oc whoami -t 2>/dev/null)
    $ grpcurl -insecure -import-path /tmp -proto Flight.proto -H 'Authorization: Bearer <token>' -d '{}' localhost:9443 arrow.flight.protocol.FlightService/ListFlights
    ```
  - You should see:
    ```console
    Unimplemented
    ```

### Verify S3 Data Connection

### Verify Python SDK
Python SDK installation and examples can be found [Python SDK](https://github.com/opendatahub-io/data-connect-hub/tree/main/sdk/python).

## Trouble Shooting