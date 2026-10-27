
DCH Autoscaling Requirements:
- The goal is to provide autoscaling to DCH rest and default flight services.
- Autoscaling is optional
- To enable autoscaling, edit DataConnectService CR and specify as per comments as follows:
```
- spec:
  flightService:
    replicas: 1
    autoScaling: # optional
      minReplicas: 1 # required
      maxReplicas: 10 # required
       pollingInterval: 15 # optional, default 15
       cooldownPeriod: 300 # optiona, default 300
       triggers:
         - name: rest_request_rate
              serverAddress: # optional, will set default by running THANOS=$(oc get route thanos-querier -n openshift-monitoring -o jsonpath='{.spec.host}')
              query: sum(rate(dch_rest_requests_total{namespace="dch-services"}[2m]))
              target:
                type: Value
                value: "0.4"

  restService:
    replicas: 1
```

- If autoScaling is specified, the DCH controller/operator follows the function scale_manual_setup in ~/DCH/dch-sup-scale.sh to create all the resources.
- Create a plan
- Once, a plan is created, review the plan to see if any potential issues
- Execute the plan
