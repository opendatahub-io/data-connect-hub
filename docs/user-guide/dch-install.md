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