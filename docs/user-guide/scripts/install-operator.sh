#!/bin/bash

OP_NAMESPACE="${1:-redhat-ods-applications}"
helm install dc-controller dc-controller/charts/ --namespace "$OP_NAMESPACE" --create-namespace --set controllerManager.image.pullPolicy=Always
