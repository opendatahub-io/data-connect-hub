
## DCH Autoscaling on OpenShift
### Prerequisites
- OpenShift CustomMetricsAutoScaling (CMA) operator
- A `DataConnectService` resource with metrics enabled for the service being
  autoscaled. See [Prometheus Metrics](metrics.md).

### Steps
- Get the OpenShift Thanos route and save the complete Prometheus server URL:
  ```bash
  THANOS_SERVER_ADDRESS="https://$(oc get route thanos-querier -n openshift-monitoring -o jsonpath='{.spec.host}')"
  printf '%s\n' "$THANOS_SERVER_ADDRESS"
  ```

- Confirm that the CMA/KEDA APIs are installed:
  ```bash
  oc get crd scaledobjects.keda.sh triggerauthentications.keda.sh
  ```

- Configure DCH REST service for autoscaling. Set `DCH_NAMESPACE` to the namespace containing
  the `DataConnectService` resource. The following shows an example with one DCH metrics. You can
  experiment with other metrics [Prometheus Metrics](metrics.md).
  ```bash
  DCH_NAMESPACE="${DCH_NAMESPACE:-dch-services}"
  THANOS_SERVER_ADDRESS="${THANOS_SERVER_ADDRESS:?Set THANOS_SERVER_ADDRESS first}"

  oc patch dataconnectservice default-dataconnectservice \
    -n "$DCH_NAMESPACE" \
    --type merge \
    --patch "$(cat <<EOF
  {
    "spec": {
      "restService": {
        "autoScaling": {
          "minReplicas": 1,
          "maxReplicas": 10,
          "pollingInterval": 15,
          "cooldownPeriod": 300,
          "triggers": [
            {
              "name": "rest_request_rate",
              "serverAddress": "$THANOS_SERVER_ADDRESS",
              "query": "sum(rate(dch_rest_requests_total[2m]))",
              "target": {
                "type": "Value",
                "value": "0.4"
              }
            }
          ]
        }
      }
    }
  }
  EOF
  )"
  ```

- Configure Flight autoscaling with the Flight request metric:
  ```bash
  DCH_NAMESPACE="${DCH_NAMESPACE:-dch-services}"
  THANOS_SERVER_ADDRESS="${THANOS_SERVER_ADDRESS:?Set THANOS_SERVER_ADDRESS first}"

  oc patch dataconnectservice default-dataconnectservice \
    -n "$DCH_NAMESPACE" \
    --type merge \
    --patch "$(cat <<EOF
  {
    "spec": {
    "flightService": {
        "autoScaling": {
          "minReplicas": 1,
          "maxReplicas": 10,
          "pollingInterval": 15,
          "cooldownPeriod": 300,
          "triggers": [
            {
              "name": "flight_request_rate",
              "serverAddress": "$THANOS_SERVER_ADDRESS",
              "query": "sum(rate(dch_flight_requests_total[2m]))",
              "target": {
                "type": "Value",
                "value": "0.4"
              }
            }
          ]
        }
      }
    }
  }
  EOF
  )"
  ```

- Verify the `DataConnectService` status:
  ```bash
  DCS_NAME="${DCS_NAME:-default-dataconnectservice}"

  oc get dataconnectservice "$DCS_NAME" \
    -n "$DCH_NAMESPACE" \
    -o wide
  ```

- Verify all autoscaling resources created for the `DataConnectService`:
  ```bash
  oc get \
    "serviceaccount/${DCS_NAME}-autoscaling-sa" \
    "secret/${DCS_NAME}-autoscaling-token" \
    "role/${DCS_NAME}-autoscaling-metrics-reader" \
     "rolebinding/${DCS_NAME}-autoscaling-metrics-reader" \
     "rolebinding/${DCS_NAME}-autoscaling-thanos-view" \
     "scaledobject.keda.sh/${DCS_NAME}-rest-autoscaling" \
    "scaledobject.keda.sh/${DCS_NAME}-flight-autoscaling" \
    "triggerauthentication.keda.sh/${DCS_NAME}-autoscaling-auth" \
    -n "$DCH_NAMESPACE" \
    -o wide
  ```

- Verify the REST and Flight `ScaledObject` targets and trigger status:
  ```bash
  oc get \
    "scaledobject.keda.sh/${DCS_NAME}-rest-autoscaling" \
    "scaledobject.keda.sh/${DCS_NAME}-flight-autoscaling" \
    -n "$DCH_NAMESPACE" \
    -o yaml
  ```

- Verify that KEDA is targeting the generated REST and Flight Deployments:
  ```bash
  oc get deployment \
    dch-rest-service \
    "dch-${DCS_NAME}-flight" \
    -n "$DCH_NAMESPACE" \
    -o wide
  ```

Every Prometheus trigger must specify `serverAddress`. DCH does not discover
or default this value.

Do not set `replicas` on a service that has `autoScaling` configured. The
controller rejects that combination because KEDA owns the Deployment replica
count while autoscaling is enabled.
