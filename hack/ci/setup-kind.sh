#!/usr/bin/env bash
# Stage 1: Create a kind cluster with Istio gateway.
set -euo pipefail
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"

kind_config_file="${CI_TEMP_DIR}/kind-config.yaml"
gateway_tls_secret="${CI_GATEWAY_NAME}-tls"

# ---------------------------------------------------------------------------
# Kind cluster
# ---------------------------------------------------------------------------

echo "=== Creating kind cluster: ${CI_KIND_CLUSTER_NAME} ==="

cat > "$kind_config_file" <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
name: ${CI_KIND_CLUSTER_NAME}
kubeadmConfigPatches:
  - |
    kind: ClusterConfiguration
    apiServer:
      extraArgs:
        service-account-issuer: https://kubernetes.default.svc
nodes:
- role: control-plane
  extraPortMappings:
  - containerPort: ${CI_GATEWAY_NODE_PORT}
    hostPort: ${CI_GATEWAY_LOCAL_PORT}
    protocol: TCP
  - containerPort: ${CI_FLIGHT_METRICS_NODE_PORT}
    hostPort: ${CI_FLIGHT_METRICS_LOCAL_PORT}
    protocol: TCP
EOF

kind create cluster --name "$CI_KIND_CLUSTER_NAME" --config "$kind_config_file"

# ---------------------------------------------------------------------------
# Namespaces
# ---------------------------------------------------------------------------

echo "=== Creating namespaces ==="
for ns in "$CI_SVC_NAMESPACE" "$CI_CONTROLLER_NAMESPACE" "$CI_GATEWAY_NAMESPACE" "$CI_TENANT_NAMESPACE" "$CI_NO_ACCESS_NAMESPACE"; do
    kubectl create ns "$ns" 2>/dev/null || true
done

# ---------------------------------------------------------------------------
# Gateway API CRDs + Istio
# ---------------------------------------------------------------------------

echo "=== Installing Gateway API CRDs ==="
kubectl apply -f "https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.3.0/standard-install.yaml"

echo "=== Installing Istio ==="
helm repo add istio https://istio-release.storage.googleapis.com/charts --force-update >/dev/null
helm repo update >/dev/null

kubectl create ns istio-system 2>/dev/null || true
helm upgrade --install istio-base istio/base -n istio-system --wait >/dev/null
helm upgrade --install istiod istio/istiod -n istio-system --wait \
    --set pilot.env.PILOT_ENABLE_GATEWAY_API=true \
    --set pilot.env.PILOT_ENABLE_GATEWAY_API_DEPLOYMENT_CONTROLLER=true >/dev/null

kubectl rollout status deployment/istiod -n istio-system --timeout=300s

# ---------------------------------------------------------------------------
# Gateway
# ---------------------------------------------------------------------------

echo "=== Creating Gateway ==="

openssl req -x509 -nodes -newkey rsa:2048 \
    -keyout "${CI_TEMP_DIR}/gateway-tls.key" \
    -out "${CI_TEMP_DIR}/gateway-tls.crt" \
    -subj "/CN=${CI_GATEWAY_NAME}.${CI_GATEWAY_NAMESPACE}.svc" \
    -addext "basicConstraints=critical,CA:FALSE" \
    -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
    -addext "extendedKeyUsage=serverAuth" \
    -addext "subjectAltName=DNS:${CI_GATEWAY_NAME}.${CI_GATEWAY_NAMESPACE}.svc,DNS:${CI_GATEWAY_NAME}.${CI_GATEWAY_NAMESPACE}.svc.cluster.local,DNS:${CI_GATEWAY_NAME}" \
    -days 365 2>/dev/null

kubectl create secret tls "$gateway_tls_secret" -n "$CI_GATEWAY_NAMESPACE" \
    --cert="${CI_TEMP_DIR}/gateway-tls.crt" \
    --key="${CI_TEMP_DIR}/gateway-tls.key" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null

kubectl apply -n "$CI_GATEWAY_NAMESPACE" -f - <<EOF
apiVersion: v1
kind: ConfigMap
metadata:
  name: ${CI_GATEWAY_NAME}-options
data:
  deployment: |
    spec:
      template:
        spec:
          containers:
            - name: istio-proxy
              resources:
                requests:
                  cpu: 50m
                  memory: 64Mi
                limits:
                  cpu: 500m
                  memory: 256Mi
EOF

kubectl apply -n "$CI_GATEWAY_NAMESPACE" -f - <<EOF
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: ${CI_GATEWAY_NAME}
spec:
  gatewayClassName: istio
  infrastructure:
    parametersRef:
      group: ""
      kind: ConfigMap
      name: ${CI_GATEWAY_NAME}-options
  listeners:
    - name: http
      port: 80
      protocol: HTTP
      allowedRoutes:
        namespaces:
          from: All
    - name: https
      port: 443
      protocol: HTTPS
      tls:
        mode: Terminate
        certificateRefs:
          - kind: Secret
            group: ""
            name: ${gateway_tls_secret}
      allowedRoutes:
        namespaces:
          from: All
EOF

# ---------------------------------------------------------------------------
# DestinationRules — Istio originates TLS to backend services
# ---------------------------------------------------------------------------

kubectl apply -n "$CI_SVC_NAMESPACE" -f - <<EOF
apiVersion: networking.istio.io/v1
kind: DestinationRule
metadata:
  name: ${CI_REST_SERVICE_NAME}-tls
spec:
  host: ${CI_REST_SERVICE_NAME}.${CI_SVC_NAMESPACE}.svc.cluster.local
  trafficPolicy:
    portLevelSettings:
      - port:
          number: 8443
        tls:
          mode: SIMPLE
          insecureSkipVerify: true
---
apiVersion: networking.istio.io/v1
kind: DestinationRule
metadata:
  name: ${CI_FLIGHT_SERVICE_NAME}-tls
spec:
  host: ${CI_FLIGHT_SERVICE_NAME}.${CI_SVC_NAMESPACE}.svc.cluster.local
  trafficPolicy:
    portLevelSettings:
      - port:
          number: 8443
        tls:
          mode: SIMPLE
          insecureSkipVerify: true
EOF

rm -f "${CI_TEMP_DIR}/gateway-tls.key" "${CI_TEMP_DIR}/gateway-tls.crt"

# ---------------------------------------------------------------------------
# Patch gateway service to use fixed NodePort (mapped via extraPortMappings)
# ---------------------------------------------------------------------------

echo "=== Patching gateway NodePort ==="
until kubectl get svc "${CI_GATEWAY_NAME}-istio" -n "$CI_GATEWAY_NAMESPACE" >/dev/null 2>&1; do
    sleep 1
done
# The Istio gateway service ports are: [0]=15021 (status), [1]=80 (http), [2]=443 (https)
kubectl patch svc "${CI_GATEWAY_NAME}-istio" -n "$CI_GATEWAY_NAMESPACE" --type='json' \
    -p="[{\"op\":\"replace\",\"path\":\"/spec/ports/2/nodePort\",\"value\":${CI_GATEWAY_NODE_PORT}}]"

echo "=== Kind cluster ready ==="
