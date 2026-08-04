# Data Connect Hub (DCH) - Installation
The purpose of this document is to provide **end-users** steps to install, configure, verify DCH as such this document can be used by doc team to build official doc. This approach is similar to other services.

## Prerequisites
- You have an OpenShift cluster on version `4.20` or higher.
- You have installed the OpenShift CLI (`oc`).
- You have installed `curl`, `grpcurl`. We will use these to test DCH REST and flight service.
- You have logged in as a user with cluster-admin privileges.
- You have installed {productname-long} {vernum}.
- A `DataScienceClusterInitialization` (DSCI) exists in your cluster. The `DataScienceClusterInitialization` gets created by the Red Hat OpenShift-AI operator out of the box. Verify DSCI as follows:
  ```
  $ oc get dsci -A

  NAME           AGE   PHASE   CREATED AT
  default-dsci   83d   Ready   2026-05-08T12:41:52Z
  ```
- A `Gateway` which will be referred to by `DataConnectService` CR. You can use `data-science-gateway` in `openshift-ingress` namespace, or point to an existing `Gateway`. `data-science-gateway` is automatically created when RHOAI operator is installed. For the purpose of this demo, we will use `data-science-gateway`.

## Install DCH Operator
### Install Manually
### Install with `DataScienceCluster` (DSC)
### Install `DataConnectService`
Once the DCH operator is running and there's an available `Gateway`, the next step is to create a `DataConnectService` which will create a REST service, a flight service, and `HttpRoute` attaching to the `Gateway`. You can create a `DataConnectService` in a namespace for a tenant as follows:

```bash
oc apply -f - <<'EOF'
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
  kind: DataConnectService
  metadata:
    name: tenant-1-data-connect
    namespace: dch-tenant-1
  spec:
    description: "Data Connect Hub for the ML platform"
    restApiReplicas: 2
    flightApiReplicas: 3
    gateway:
      name: data-science-gateway
      namespace: openshift-ingress
EOF
```
### Verify `DataConnectService`
You can verify the `DataConnectService` as follows:
- Verify all pods are up and running:
  ```
  $ oc get po -n dch-tenant-1 -l app.kubernetes.io/part-of=data-connect-hub
  NAME                              READY   STATUS    RESTARTS   AGE
  flight-service-789d77c878-m94nh   1/1     Running   0          46m
  postgres-7f46bcbd7b-77t5h         1/1     Running   0          67m
  rest-service-7f4cc4948b-n6llb     1/1     Running   0          73m
  ```
- Verify the REST service in the pod is responding:
  ```
  $ oc exec -it $(oc get po -n dch-tenant-1 -l app.kubernetes.io/name=rest-service -o jsonpath='{.items[0].metadata.name}') -n dch-tenant-1 -- curl -s http://localhost:8080/api/v1/data/connections
  ```

- Verify the flight service in the pod is responding by running the following script:
  ```bash
  #!/bin/bash
  flight_pod=$(oc get po -n dch-tenant-1 -l app.kubernetes.io/name=flight-service -o jsonpath='{.items[0].metadata.name}')

  curl -sL -o /tmp/Flight.proto https://raw.githubusercontent.com/apache/arrow/main/format/Flight.proto
  
  oc port-forward $flight_pod -n dch-tenant-1 50051:50051 &

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