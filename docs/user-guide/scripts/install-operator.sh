#!/bin/bash

OP_NAMESPACE="${1:-redhat-ods-applications}"

echo helm install dc-controller dc-controller/charts/ \
    --namespace "$OP_NAMESPACE" --create-namespace \
    --set controllerManager.image.pullPolicy=Always

helm install dc-controller dc-controller/charts/ \
    --namespace "$OP_NAMESPACE" --create-namespace \
    --set controllerManager.image.pullPolicy=Always
