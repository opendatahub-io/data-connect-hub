# Vault integration

## Current approach

The current proof of concept uses one Vault configuration shared by the REST
and Flight services. Tenant isolation is provided by constructing paths under
`dch/{tenant_id}`. Vault credentials are read directly and are not copied into
Kubernetes Secrets.

### Requirements

- A reachable Vault server using HTTPS and KV v2
- A trusted CA certificate available to both DCH services
- A Vault authentication role for the REST and Flight service accounts
- A projected service-account token with audience `vault` in both deployments

### Setup

1. Enable and configure Vault Kubernetes authentication.
2. Create a policy for the DCH tenant hierarchy and bind it to the DCH service
  accounts:
3. Store credentials under the tenant-specific path. Credential names must
  match the connection type schema; PostgreSQL expects `URI`:
4. Configure `spec.vault` on the `DataConnectService`. The controller writes
   this service-level configuration into the REST and Flight `config.toml`
   ConfigMaps. It describes how DCH authenticates to Vault, not any
   connector-specific credentials.
5. When Vault uses a custom CA, create a ConfigMap containing the PEM file and
   reference its name and key with `spec.vault.caConfigMap`. The controller
   mounts it at `/etc/tls/vault/ca.crt`. The base deployments project a Vault
   service-account token at `/var/run/secrets/vault/token`.
6. Create a connection with a relative Vault path:
  ```json
   {
     "credentials_ref": {
       "vault": {
         "path": "postgres/demo"
       }
     }
   }
  ```
7. Run the connection readiness check and a query. Confirm that no Kubernetes
  Secret contains the data-source credentials.

This model trusts DCH to enforce tenant path construction. The shared Vault
role can currently read the configured `dch/*` hierarchy.

The Vault secret itself is connector-agnostic. Each KV v2 document contains
the fields required by the selected connection type, for example `URI` for a
PostgreSQL connection or the S3 access fields for an S3 connection.

```yaml
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
spec:
  vault:
    address: https://vault.example:8200
    role: dch
    kvMount: secret
    authMount: kubernetes
    tenantPrefix: dch
    caConfigMap:
      name: vault-ca
      key: ca.crt
```

## Further investigation

Supporting a separate Vault for each tenant requires a provider model rather
than additional static `[vault]` sections.

### Tenant-scoped providers

Introduce a tenant-owned Vault provider resource containing non-secret
configuration such as:

- Vault address, KV mount, and authentication mount
- Vault role or workload identity
- CA bundle reference
- Allowed path prefixes

A connection would reference both a provider and a relative secret path:

```json
{
  "credentials_ref": {
    "vault": {
      "provider": "tenant-vault",
      "path": "postgres/demo"
    }
  }
}
```

DCH should validate provider ownership, construct paths server-side, and cache
clients and short-lived tokens by tenant and provider. Tokens and resolved
credentials must not be persisted in metadata.

### External Vaults

External Vault support must account for:

- DNS, firewall, proxy, and NetworkPolicy egress
- TLS hostname validation and tenant-managed CA rotation
- An authentication mechanism that does not use long-lived static tokens
- Whether an external Vault can reach the Kubernetes TokenReview API when
Kubernetes authentication is used

Possible authentication mechanisms include Kubernetes authentication, OIDC,
or cloud workload identity. The appropriate mechanism depends on where Vault
is hosted and the tenant's trust model.

### Access control

Each tenant provider should use a dedicated Vault role and narrowly scoped
policy. Avoid policies such as `secret/data/*`:

```hcl
path "tenant-a/data/data-connect/postgres/*" {
  capabilities = ["read"]
}
```

Vault KV policies control access to paths, not individual fields inside one KV
document. Credentials requiring different permissions must therefore be stored
at separate paths. Do not grant `list` unless discovery is explicitly needed.

Further design should cover provider onboarding authorization, role and CA
rotation, revocation, audit logging, failure isolation, and tests proving that
one tenant cannot select another tenant's provider or path.
