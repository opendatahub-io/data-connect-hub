
## DCH Trace
### Prerequisites
- Redhat Tempo Operator
- TempoMonolithic resource. An example TempoMonolithic:
  ```
  kubectl apply -n redhat-ods-monitoring -f - <<'EOF'
  apiVersion: tempo.grafana.com/v1alpha1
  kind: TempoMonolithic
  metadata:
    name: dch-trace
  spec:
    storage:
      traces:
        backend: memory
    jaegerui:
      enabled: true
      route:
        enabled: true
    ingestion:
      otlp:
        grpc:
          enabled: true
          tls:
            enabled: true
            minVersion: "1.2"
        http:
          enabled: true
          tls:
            enabled: true
            minVersion: "1.2"
    EOF
  ```
  
### Steps
- Get the exporter URL, for example:
  ```
  kubectl get svc -n redhat-ods-monitoring tempo-dch-trace
  NAME              TYPE        CLUSTER-IP       EXTERNAL-IP   PORT(S)                      AGE
  tempo-dch-trace   ClusterIP   172.30.102.113   <none>        3200/TCP,4317/TCP,4318/TCP   23h
  ```

- Configure DCH service to enable trace:
  ```
  kubectl patch dataconnectservice default-dataconnectservice -n dch-services --type merge -p '{
      "spec": {"trace": {
        "exporter": "https://tempo-trace.redhat-ods-monitoring.svc.cluster.local:4317",
        "insecure": false,
        "certificate": "/var/run/secrets/kubernetes.io/serviceaccount/service-ca.crt"
      }}}'
  ```

- Get Jaeger UI route, for example:

  ```
  kubectl get route -n redhat-ods-monitoring
  NAME                       HOST/PORT                                                                          PATH   SERVICES                   PORT          TERMINATION   WILDCARD
  tempo-dch-trace-jaegerui   tempo-dch-trace-jaegerui-redhat-ods-monitoring.apps......ibm.com          tempo-dch-trace-jaegerui   oauth-proxy   reencrypt     None
  ```

- Send requests to REST and Flight services.
- Use browser to connect to jaeger UI route. You should see 2 services `dch-rest-service`, `dch-flight-service`, and you can start to find traces.