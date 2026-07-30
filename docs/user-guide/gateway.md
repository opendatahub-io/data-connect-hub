## DCH Service Instance Gateway
- Ref: [Architecture Decision Record](https://github.com/mariusdanciu/architecture-decision-records/blob/data-connect-hub/architecture-decision-records/data-connect-hub/ODH-ADR-0001-data-connect-hub.md)
  - For soft tenancy, there's 1 gateway in `openshift-ingress` namespace for all tenants, and there are multiple HttpRoutes from tenant infra namespaces pointing to this gateway.
  - For hard tenancy, each tenant has its own gateway, all in `openshift-ingress` namespace, and HttpRoute from each tenant pointing to the corresponding gateway.
  - 

- The following shows how LLM inference service support gateways:
  - Use a default or known gateway
  - Point to an existing gateway
  
## Gateway for LLM Inference Service
### Use a default or known gateway
Here are the steps:
  - User creates a gateway:
    ```
    apiVersion: gateway.networking.k8s.io/v1
    kind: GatewayClass
    metadata:
    name: openshift-default
    spec:
    controllerName: openshift.io/gateway-controller/v1
    ---
    apiVersion: gateway.networking.k8s.io/v1
    kind: Gateway
    metadata:
    name: openshift-ai-inference    <=== name
    namespace: openshift-ingress   <=== namespace
    spec:
    gatewayClassName: openshift-default
    listeners:
        - name: http
        port: 80
        protocol: HTTP
        allowedRoutes:
            namespaces:
            from: All
    ``` 
    ```
    $ k get gateway -n openshift-ingress openshift-ai-inference
    NAME                     CLASS               ADDRESS                                                                        PROGRAMMED   AGE
    openshift-ai-inference   openshift-default   openshift-ai-inference-openshift-default.openshift-ingress.svc.cluster.local   True         117d
    ```
  - Specify empty gateway in `llmisvc` CR:
    ```
    kind: LLMInferenceService
    spec:
        router:
            gateway: {} <=== empty
    ```

  - `llmisvc` automatically fill in the gateway in status section:
    ```
    status:
        gateways:
        - group: gateway.networking.k8s.io
          httpRoutes:
          - group: gateway.networking.k8s.io
             kind: HTTPRoute
                name: qwen2-7b-instruct-pd-kserve-route
                namespace: autoscaling-example  
        kind: Gateway
        name: openshift-ai-inference  <=== 
        namespace: openshift-ingress
    ```
  - HTTPRoute created by `llmisvc`:
    ```
    kind: HTTPRoute
    metadata:
    creationTimestamp: "2026-07-09T21:06:57Z"
    generation: 1
    labels:
        app.kubernetes.io/component: llminferenceservice-router
        app.kubernetes.io/name: qwen2-7b-instruct-pd
        app.kubernetes.io/part-of: llminferenceservice
    name: qwen2-7b-instruct-pd-kserve-route
    namespace: autoscaling-example
    ownerReferences:
    - apiVersion: serving.kserve.io/v1alpha2
        blockOwnerDeletion: true
        controller: true
        kind: LLMInferenceService           <===
        name: qwen2-7b-instruct-pd
    spec:
    parentRefs:
    - group: gateway.networking.k8s.io
        kind: Gateway            <===
        name: openshift-ai-inference <===
        namespace: openshift-ingress
        ```
  - `llmisvc`:
    ```
    $ k get llmisvc -n autoscaling-example qwen2-7b-instruct-pd
    NAME                   URL                                                                                                                             READY   REASON                       AGE
    qwen2-7b-instruct-pd   https://openshift-ai-inference-openshift-default.openshift-ingress.svc.cluster.local/autoscaling-example/qwen2-7b-instruct-pd   False   MinimumReplicasUnavailable   20d
    ```   
### User points to an existing gateway
An existing gateway may be created like this:
  ```
  apiVersion: v1
kind: ConfigMap
metadata:
  name: autoscaling-example-gateway-config
  namespace: autoscaling-example       
data:
  service: |
    metadata:
      annotations:
        service.beta.openshift.io/serving-cert-secret-name: "autoscaling-example-gateway-tls"
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
                  cpu: "16"
                  memory: 16Gi
                requests:
                  cpu: "4"
                  memory: 4Gi
---
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: autoscaling-example-gateway
  namespace: autoscaling-example         <=== not openshift-ingress
spec:
  gatewayClassName: data-science-gateway-class <== not openshift-default
  infrastructure:
    parametersRef:
      group: ""
      kind: ConfigMap
      name: autoscaling-example-gateway-config
  listeners:
  - allowedRoutes:
      namespaces:
        from: Same
    name: https
    port: 443
    protocol: HTTPS
    tls:
      certificateRefs:
      - group: ""
        kind: Secret
        name: autoscaling-example-gateway-tls
      mode: Terminate

  ```

  LLM inference service can point to the gateway:
  ```
    router:
    scheduler: { }
    route: { }
    gateway:
      refs: # Reference the gateway we created
      - name: autoscaling-example-gateway         <===
        namespace: autoscaling-example
  ```

Notes:
- LLM Inference service doesn't use to `data-science-gateway` in `openshift-ingress` namespace (at least by default). This gateway is automatically created when RHOAI operator is installed.
- LLM Inference service can use a default or known gateway `openshift-ai-inference` in `openshift-ingress` namespace, or a gateway in a namespace can be specified.

## Gateway for DCH Service Instance
Options for DCH:
- If gateway is not specified:
  - Use `data-science-gateway` in `openshift-ingress`. If we don't want to use the default `data-science-gateway` then we can default to a specific name such as `openshift-data-connect-hub` gateway in `openshift-ingress` - this is similar to LLM inference service above.
  - A gateway for all DCH service instances is the `soft tenancy` model.
- If a gateway is specified:
  - Use a gateway in `openshift-ingress`. This is also similar to LLM inference service above.
  - A gateway for a DCH service instance is the `multi tenancy` model.

