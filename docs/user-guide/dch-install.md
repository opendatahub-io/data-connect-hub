# Data Connect Hub (DCH) - Installation
The purpose of this document is to provide **end-users** steps to install, configure, verify DCH as such this document can be used by doc team to build official doc. This approach is similar to other services.

## Content
- Prerequisites
- Install DCH Operator
- Install `DataConnectService`
- Verify `DataConnectService`
- DCH Rest Swagger
- Verify DCH REST Service
- Verify Flight Service
- Verify S3 Data Connection
- DCH Python SDK
- Trouble Shooting

## Prerequisites
- You have an OpenShift cluster on version `4.20` or higher.
- You have installed the OpenShift CLI (`oc`).
- You have installed `curl`, `grpcurl`. We will use these to test DCH REST and flight service. There are different versions of `grpcurl` and they work differently. `grpcurl` used in this document was installed with `go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest`.
- You have logged in as a user with cluster-admin privileges.
- You have installed {productname-long} {vernum}.
- A `DataScienceClusterInitialization` (DSCI) exists in your cluster. The `DataScienceClusterInitialization` gets created by the Red Hat OpenShift-AI operator out of the box. Verify DSCI as follows:
  ```
  $ oc get dsci -A

  NAME           AGE   PHASE   CREATED AT
  default-dsci   83d   Ready   2026-05-08T12:41:52Z
  ```
- A `Gateway` which will be referred to by `DataConnectService` CR. You can use an existing `Gateway`. For the purpose of this demo, we will create a `Gateway` called `dch-gateway` in `openshift-ingress` namespace as follows:
  ```
  $ oc apply -f - <<EOF
  apiVersion: v1
  kind: ConfigMap
  metadata:
    name: dch-gateway-config
    namespace: openshift-ingress
  data:
    service: |
      metadata:
        annotations:
          service.beta.openshift.io/serving-cert-secret-name: "dch-gateway-tls"
      spec:
        type: ClusterIP
    deployment: |
      spec:
        template:
          spec:
            containers:
              - name: istio-proxy
                resources:
                  limits:
                    cpu: "2"
                    memory: 2Gi
                  requests:
                    cpu: 500m
                    memory: 512Mi
  ---
  apiVersion: gateway.networking.k8s.io/v1
  kind: Gateway
  metadata:
    name: dch-gateway
    namespace: openshift-ingress
  spec:
    gatewayClassName: data-science-gateway-class
    infrastructure:
      parametersRef:
        group: ""
        kind: ConfigMap
        name: dch-gateway-config
    listeners:
    - allowedRoutes:
        namespaces:
          from: All
      name: https
      port: 443
      protocol: HTTPS
      tls:
        certificateRefs:
        - group: ""
          kind: Secret
          name: dch-gateway-tls
        mode: Terminate
  EOF
  }
    ```
- A namespace called `dch-example` for this demo. You can create a namespace as follows:
  ```
  $ oc new-project dch-example
  ```

## Install DCH Operator
### Install Manually
### Install with `DataScienceCluster` (DSC)
  - Post DP
### Install `DataConnectService`
Once the DCH operator is running and there's an available `Gateway`, the next step is to create a `DataConnectService` which will create a REST service, a flight service, and `HttpRoute` attaching to the `Gateway`. You can create a `DataConnectService` in a namespace for a tenant as follows:

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

### DCH Rest Swagger
   - TODO

### Verify DCH REST Service
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
### Verify Flight Service
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

## Trouble Shooting