# Data Connect Hub (DCH) - Installation
The purpose of this document is to provide **end-users** steps to install, configure, verify DCH as such this document can be used by doc team to build official doc. This approach is similar to other services.

## Prerequisites
- You have an OpenShift cluster on version `4.20` or higher.
- You have installed the OpenShift CLI (`oc`).
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
- TODO