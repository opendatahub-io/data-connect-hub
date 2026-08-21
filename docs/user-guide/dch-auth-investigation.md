# DCH Authentication Investigation
## HttpRoute:
- Route:
  ```
  apiVersion: gateway.networking.k8s.io/v1
    kind: HTTPRoute
    ...
    spec:
    parentRefs:
    - group: gateway.networking.k8s.io
        kind: Gateway
        name: dch-gateway
        namespace: openshift-ingress
    rules:
    - backendRefs:
        - group: ""
            kind: Service
            name: rest-service  <-- rest service
            port: 8443  <--- kube-rbac-proxy pod listening
            weight: 1
        matches:
        - path:
            type: PathPrefix
            value: /api/v1
    - backendRefs:
        - group: ""
            kind: Service
            name: flight-service  <-- flight service
            port: 8443 <--- kube-rbac-proxy pod listening
            weight: 1
        matches:
        - path:
            type: PathPrefix
            value: /arrow.flight.protocol.FlightService
    ```
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

## Requests:
  - $ user_token=$(oc whoami -t 2>/dev/null)
  - $ curl -sk -H "Authorization: Bearer $user_token" -H 'x-tenant-id: dch-example' https://localhost:8443/api/v1/data/connections
  - $ grpcurl -insecure -import-path /tmp -proto Flight.proto -H "Authorization: Bearer $user_token" -H 'x-tenant-id: dch-example'  -d '{}' localhost:9443 arrow.flight.protocol.FlightService/ListFlights
- The gateway forwards to
  the backend over TLS (kube-rbac-proxy's self-signed cert).

## Flow
  1. Client sends a request with Authorization: Bearer <token> header (a Kubernetes service account or user token).

  2. kube-rbac-proxy intercepts — it listens on port 8443 and does two things:
     - Authentication: Sends a TokenReview to the Kubernetes API to validate the bearer token
     - Authorization: Sends a SubjectAccessReview to check if the authenticated user has permission
  3. Authorization check — configured via static rules in each service's ConfigMap:
  4.  If authorized, kube-rbac-proxy forwards the request to the upstream service on localhost:
      - rest-service: http://127.0.0.1:8080 (HTTP/1.1)
      - flight-service: http://127.0.0.1:50051 (HTTP/2 via --upstream-force-h2c=true)

  5. If unauthorized, kube-rbac-proxy returns 401 Unauthorized — the request never reaches the service

## Kube-rbac-proxy Authentication

### Sidecar container (in each Deployment)

    rest-service:
    - name: kube-rbac-proxy
        ```
        image: quay.io/brancz/kube-rbac-proxy:v0.18.1
        args:
        - "--secure-listen-address=0.0.0.0:8443"
        - "--upstream=http://127.0.0.1:8080/"       # REST on HTTP/1.1
        - "--auth-header-fields-enabled=true"
        - "--upstream-force-h2c=false"
        - "--config-file=/etc/kube-rbac-proxy/config.yaml"
        - "--logtostderr=true"
        - "--v=2"
        ports:
        - name: https
            containerPort: 8443
        volumeMounts:
        - name: rbac-proxy-config
            mountPath: /etc/kube-rbac-proxy
        ```

    flight-service:
    - name: kube-rbac-proxy
        ```
        image: quay.io/brancz/kube-rbac-proxy:v0.18.1
        args:
        - "--secure-listen-address=0.0.0.0:8443"
        - "--upstream=http://127.0.0.1:50051/"      # gRPC on HTTP/2
        - "--auth-header-fields-enabled=true"
        - "--upstream-force-h2c=true"                # force HTTP/2 cleartext
        - "--config-file=/etc/kube-rbac-proxy/config.yaml"
        - "--logtostderr=true"
        - "--v=2"
        ports:
        - name: https
            containerPort: 8443
        volumeMounts:
        - name: rbac-proxy-config
            mountPath: /etc/kube-rbac-proxy
        ```

### Authorization config (ConfigMap per service)

  - rest-service-rbac-proxy-config CM:
    ```
    authorization:
        static:
        - resourceRequest: true
          resource: rest-service
          apiGroup: dh.io
          verb: use
    ```
  - flight-service-rbac-proxy-config CM:
    ```
    authorization:
        static:
        - resourceRequest: true
          resource: flight-service
          apiGroup: dh.io
          verb: use
    ```

### Service (exposes only the proxy port)
- Service
  ```
  ports:
    - name: https
      port: 8443
      targetPort: 8443
    ```
  Both services expose only port 8443 (kube-rbac-proxy). The actual service ports (8080, 50051) are internal to the pod.

### Service accounts
- rest-service deployment
     ```
     serviceAccountName: data-connect-hub-sa
     automountServiceAccountToken: true
     ```

- flight-service deployment
     ```
     serviceAccountName: flight-service-sa
     automountServiceAccountToken: true
     ```

automountServiceAccountToken: true is required so the proxy can call the Kubernetes API for TokenReview/SubjectAccessReview.

### RBAC resources
- Allow service accounts to call TokenReview and SubjectAccessReview
    ```
        apiVersion: rbac.authorization.k8s.io/v1
        kind: ClusterRole
        metadata:
            name: dch-rbac-proxy
        rules:
        - apiGroups: ["authentication.k8s.io"]
            resources: ["tokenreviews"]
            verbs: ["create"]
        - apiGroups: ["authorization.k8s.io"]
            resources: ["subjectaccessreviews"]
            verbs: ["create"]
        ---
        # Bind both service accounts
        apiVersion: rbac.authorization.k8s.io/v1
        kind: ClusterRoleBinding
        metadata:
            name: dch-rbac-proxy
        subjects:
        - kind: ServiceAccount
            name: data-connect-hub-sa    <=== service account
            namespace: <namespace>
        - kind: ServiceAccount
            name: flight-service-sa  <=== service account
            namespace: <namespace>
        roleRef:
            kind: ClusterRole
            name: dch-rbac-proxy
        ```
### User access Roles (per service)
- Grant users permission to use rest-service
    ```
        apiVersion: rbac.authorization.k8s.io/v1
        kind: Role
        metadata:
            name: dh-rest-service-access
        rules:
        - apiGroups: ["dh.io"]
            resources: ["rest-service"]
            verbs: ["use"]
        ---
        apiVersion: rbac.authorization.k8s.io/v1
        kind: RoleBinding
        metadata:
            name: dh-rest-service-access-shuynh
        subjects:
        - kind: User
            name: shuynh@redhat.com       <=========  grant shuynh user
        roleRef:
            kind: Role
            name: dh-rest-service-access
        ---
        # Grant users permission to use flight-service
        apiVersion: rbac.authorization.k8s.io/v1
        kind: Role
        metadata:
            name: dh-flight-service-access
        rules:
        - apiGroups: ["dh.io"]
            resources: ["flight-service"]
            verbs: ["use"]
        ---
        apiVersion: rbac.authorization.k8s.io/v1
        kind: RoleBinding
        metadata:
            name: dh-flight-service-access-shuynh
        subjects:
        - kind: User
            name: shuynh@redhat.com
        roleRef:
            kind: Role
            name: dh-flight-service-access
    ```

- Tests:
  ```
    Test 1: Unauthenticated gRPC request (expect Unauthenticated)...
    CMD: grpcurl -insecure -import-path /tmp -proto Flight.proto -d '{}' localhost:9443 arrow.flight.protocol.FlightService/ListFlights
    Handling connection for 9443
    RESPONSE: ERROR:
    Code: Unauthenticated
    Message: unexpected HTTP status code received from server: 401 (Unauthorized); transport: received unexpected content-type "text/plain; charset=utf-8"
    PASSED: unauthenticated request rejected

    Test 2: Bad token gRPC request (expect Unauthenticated)...
    CMD: grpcurl -insecure -import-path /tmp -proto Flight.proto -H 'Authorization: Bearer bad-token' -d '{}' localhost:9443 arrow.flight.protocol.FlightService/ListFlights
    Handling connection for 9443
    RESPONSE: ERROR:
    Code: Unauthenticated
    Message: unexpected HTTP status code received from server: 401 (Unauthorized); transport: received unexpected content-type "text/plain; charset=utf-8"
    PASSED: bad token rejected

    Test 3: Authenticated ListFlights RPC call...
    User: shuynh@redhat.com
    CMD: grpcurl -insecure -import-path /tmp -proto Flight.proto -H 'Authorization: Bearer sha256~V773RWeib50ss.........NAHJhnnmmtwir81oXtBYM' -H 'x-tenant-id: dch-example' -d '{}' localhost:9443 arrow.flight.protocol.FlightService/ListFlights
    Handling connection for 9443
    RESPONSE: ERROR:
    Code: Unimplemented
    Message: Not yet implemented
    PASSED: authenticated ListFlights responded

    Test 4: Non-matching gRPC path (expect failure)...
    CMD: grpcurl -insecure -import-path /tmp -proto Flight.proto -H 'Authorization: Bearer sha256~V7.....XtBYM' -H 'x-tenant-id: dch-example' -d '{}' localhost:9443 fake.service.BadPath/BadMethod
    Handling connection for 9443
    RESPONSE: Error invoking method "fake.service.BadPath/BadMethod": target server does not expose service "fake.service.BadPath"
    PASSED: non-matching path rejected    
  ```