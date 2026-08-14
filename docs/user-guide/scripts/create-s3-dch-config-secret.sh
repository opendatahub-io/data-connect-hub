#!/bin/bash

NAMESPACE="${1:-dch-example}"

kubectl create secret generic ${DCH_S3_SECRET_NAME} \
    -n $NAMESPACE \
    --from-literal=AWS_S3_ENDPOINT=${AWS_S3_ENDPOINT} \
    --from-literal=AWS_DEFAULT_REGION=${AWS_DEFAULT_REGION} \
    --from-literal=AWS_S3_BUCKET=${AWS_S3_BUCKET} \
    --from-literal=AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID} \
    --from-literal=AWS_SECRET_ACCESS_KEY=${AWS_SECRET_ACCESS_KEY} \
    --dry-run=client -o yaml | kubectl apply -f -
