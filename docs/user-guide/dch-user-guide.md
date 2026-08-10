# Data Connect Hub (DCH) - User Guide
The purpose of this document is to provide **end-users** steps to install, configure, use DCH as such this document can be used by doc team to build official doc. This approach is similar to other services.

## Content
- [x] Prerequisites
- [ ] Install DCH Operator
- [ ] Install `DataConnectService`
- [ ] Verify `DataConnectService`
- [x] Create and Grant Users to DCH Service
- [ ] Auto Migrate Existing RHAI Connections and Connection Types
- [ ] Create Connections and Connection Types
- [ ] DCH Rest Swagger
- [ ] DCH REST Service
- [ ] DCH Flight Service
- [ ] Flight Service jsonl files ingestion - S3
- [ ] Verify S3 Data Connection
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
- A `Gateway` which will be referred to by `DataConnectService` CR. You can use an existing `Gateway`. For the purpose of this demo, we will create a `Gateway` called `dch-gateway` in `openshift-ingress` namespace by running the [scripts/create-gateway.sh](scripts/create-gateway.sh)
 
- A namespace called `dch-example` for this demo. You can create a namespace as follows:
  ```
  $ oc new-project dch-example
  ```

## Install DCH Operator
### Install with `Helm`
- For Dev Preview (DP), you can install the operator as follows:
  - Clone the repo `https://github.com/red-hat-data-services/data-connect-hub`
  - Change directory to `data-connect-hub`.
  - Run the [scripts/install-operator.sh](scripts/install-operator.sh). This script installs the operator in `redhat-ods-applications` namespace.
- You can use the following commands to delete the `helm` chart:
  ```bash
    $ helm delete dc-controller -n redhat-ods-applications --no-hooks
    $ oc delete secret sh.helm.release.v1.dc-controller.v1 -n redhat-ods-applications
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
  flight-service-789d77c878-m94nh   1/1     Running   0          46m
  postgres-7f46bcbd7b-77t5h         1/1     Running   0          67m
  rest-service-7f4cc4948b-n6llb     1/1     Running   0          73m
  ```
- Verify the REST service in the pod is responding:
  ```
  $ oc exec -it $(oc get po -n dch-example -l app.kubernetes.io/name=rest-service -o jsonpath='{.items[0].metadata.name}') -n dch-example -- curl -s http://localhost:8080/api/v1/data/connections
  ```

- Verify the flight service in the pod is responding by running the following script:
  ```bash
  #!/bin/bash
  flight_pod=$(oc get po -n dch-example -l app.kubernetes.io/name=flight-service -o jsonpath='{.items[0].metadata.name}')

  curl -sL -o /tmp/Flight.proto https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto
  
  oc port-forward $flight_pod -n dch-example 50051:50051 &

  grpcurl -plaintext -import-path /tmp -proto Flight.proto localhost:50051 list

  grpcurl -plaintext -import-path /tmp -proto Flight.proto localhost:50051 describe arrow.flight.protocol.FlightService
  ```

  You should see the following output:
  ```console
  arrow.flight.protocol.FlightService
  arrow.flight.protocol.FlightService is a service:
  // A flight service is an endpoint for retrieving or storing Arrow data. A
  // flight service can expose one or more predefined endpoints that can be
  // accessed using the Arrow Flight Protocol. Additionally, a flight service
  // can expose a set of actions that are available.
  service FlightService {
  ...
  ```

### Create and Grant Users to DCH Service
For the purpose of the demo, we create `serviceaccount` instead of users.
- To create and grant a user who can perform DCH **read**, run the script [create-grant-test-user.sh](scripts/create-grant-test-user.sh). You should see the following output:
   ```console
   TBD
  ```
- To create and grant an admin who can perform DCH **read/write**, run the script [create-grant-admin-user.sh](scripts/create-grant-admin-user.sh). You should see the following output:
   ```console
   TBD
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