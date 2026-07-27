# DCH - Installation Procedure for Tech Preview
The purpose of this document is to provide **end-users** steps to install, configure, verify DCH as such this document can be used by doc team to build official doc. This approach is similar to other services such as WVA, distributed tracing, for example.

## Prerequisites
- You have an OpenShift cluster on version `4.20` or later.
- You have installed the OpenShift CLI (`oc`).
- You have logged in as a user with cluster-admin privileges.



## Installation & Verification Steps
  - Create Tenants & Namespaces
  - Install Postgresql
  
## Create Users & Namespaces
For each DCH tenant, there is an `infra` namespace and an `admin` namespace.
  - Create a tenant infra namespace called `dch-tenant-1-infra`, and a tenant user `dch-tenant-1` for the namespace as follows:
    - Create namespace:
        ```console
        oc new-project dch-tenant-1-infra
        ```
  - Create a tenant admin namespace called `dch-tenant-1-admin`, and a tenant admin `dch-tenant-1-admin` for the namespace as follows:
    - Create namespace:
        ```console
        oc new-project dch-tenant-1-admin
        ```

### Install Postgresql in Tenant Infra Namespace
For each DCH tenant, an instance of Postgresql is expected to be **pre-installed** in the tenant infra namespace [Marius: pls confirm]. In this section, we show an example of how to install an instance of Postgresql in an Openshift cluster such as a ROSA cluster.

- You should be log into the cluster as `dch-tenant-1`
- Create Postgresql instance:
  ```console
  oc process postgresql-persistent -n openshift \
      -p POSTGRESQL_USER=dch_user \
      -p POSTGRESQL_PASSWORD=dch_password \
      -p POSTGRESQL_DATABASE=dch_db \
      -p VOLUME_CAPACITY=5Gi | oc apply -n dch-tenant-1-infra -f -
  ```

- Here's an example of pod listing:
  ```console
    oc get pods -n dch-tenant-1-infra

    NAME                  READY   STATUS      RESTARTS   AGE
    postgresql-1-deploy   0/1     Completed   0          2m11s
    postgresql-1-jnczk    1/1     Running     0          2m10s
  ```
- Here's an example of PVC listing:
  ```console
    oc get pvc -n dch-tenant-1-infra
    NAME         STATUS   VOLUME                                     CAPACITY   ACCESS MODES   STORAGECLASS   VOLUMEATTRIBUTESCLASS   AGE
    postgresql   Bound    pvc-6e5614b0-45df-47a9-83bf-119942f30e0e   5Gi        RWO            gp3-csi        <unset>                 2m31s
  ```
- Forward database service to local host:
  ```console
    oc port-forward svc/postgresql 5432:5432 -n dch-tenant-1-infra &
  ```
- Here's an example of listing tables:
  ```console
  PGPASSWORD=dch_password psql -h localhost -U dch_user -d dch_db -c '\dt'

        List of relations
    Schema |  Name   | Type  |  Owner   
    --------+---------+-------+----------
    public | prompts | table | dch_user
    (1 row)                                           
  ```

## Verify REST Connection Service
REST connection service is only available from within the cluster, i.e., there's no public route to the service.
- Forward REST service to local host:
  ```console
  ```

- Get all connections:
  ```
  curl -s http://localhost:8080/v1/data/connections
  ```

  You should see:
  ```
  TODO
  ```
  
- Get all connections in a namespace, for example, `dch-tenant-1`:
  ```
  curl -s http://localhost:8080/v1/data/connections/dch-tenant-1
  ```

  You should see:
  ```
  TODO
  ```

- Get a connection in a namespace, for example, `dch-tenant-1`:
  ```
  curl -s http://localhost:8080/v1/data/connections/dch-tenant-1/test-name
  ```

  You should see:
  ```
  TODO
  ```

## Verify Flight Service