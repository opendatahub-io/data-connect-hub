"""Using a token provider for automatic token refresh.

For short-lived tokens (e.g., OpenShift/OIDC), use ``token_provider``
instead of a static ``token``.  The SDK calls your function once, caches
the result, and auto-refreshes when a request receives a 401.

Usage:
    python examples/token_provider.py

Requires:
    - Execution inside a Kubernetes Pod (reads the in-cluster SA token)
    - A running DCH gateway
    - pip install httpx  (already a dependency of the SDK)

Set environment variables to override defaults:
    DCH_HOST, DCH_TENANT_ID,
    K8S_NAMESPACE, K8S_SERVICE_ACCOUNT, K8S_SA_ISSUER,
    K8S_API, K8S_CA_CERT
"""

import os

import httpx

from data_connect_hub import DataConnectClient

# Update these to match your cluster and service account configuration.
NAMESPACE = os.getenv("K8S_NAMESPACE", "default")
SERVICE_ACCOUNT = os.getenv("K8S_SERVICE_ACCOUNT", "default")
SA_ISSUER = os.getenv("K8S_SA_ISSUER", "https://kubernetes.default.svc")
K8S_API = os.getenv("K8S_API", "https://kubernetes.default.svc")
TOKEN_PATH = "/var/run/secrets/kubernetes.io/serviceaccount/token"


def get_k8s_token() -> str:
    """Request a short-lived service account token via the K8s TokenRequest API."""
    with open(TOKEN_PATH) as f:
        sa_token = f.read().strip()

    resp = httpx.post(
        f"{K8S_API}/api/v1/namespaces/{NAMESPACE}/serviceaccounts/{SERVICE_ACCOUNT}/token",
        headers={"Authorization": f"Bearer {sa_token}"},
        json={
            "apiVersion": "authentication.k8s.io/v1",
            "kind": "TokenRequest",
            "spec": {
                "audiences": [SA_ISSUER],
                "expirationSeconds": 3600,
            },
        },
        verify=os.getenv("K8S_CA_CERT", True),
    )
    resp.raise_for_status()
    return resp.json()["status"]["token"]


client = DataConnectClient(
    endpoint=os.getenv("DCH_HOST", "localhost:8443"),
    token_provider=get_k8s_token,
    tenant_id=os.getenv("DCH_TENANT_ID", "default"),
)

# The first call triggers the provider; subsequent calls use the cached token.
connections = client.list_connections()
print(f"Found {len(connections)} connection(s):")
for conn in connections:
    print(f"  [{conn.id}] {conn.name}")

# If the token expires mid-session, the SDK automatically calls
# get_k8s_token() again and retries the request.
types = client.list_connection_types()
print(f"\nFound {len(types)} connection type(s):")
for ct in types:
    print(f"  [{ct.id}] {ct.name}")
