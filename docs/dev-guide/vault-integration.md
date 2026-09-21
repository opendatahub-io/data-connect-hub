# Vault integration POC

## Objective

Prove that Data Connect Hub can resolve connector credentials directly from
HashiCorp Vault without first materializing them as Kubernetes Secrets.

## Scope

- HashiCorp Vault KV v2 static secrets
- Vault Kubernetes authentication
- PostgreSQL as the demonstration connector
- Existing Kubernetes Secret references remain supported
- Optional KV secret version selection
- Vault credentials are not exported to Kubernetes Secrets

Dynamic database secrets, lease renewal for generated database credentials,
additional secret providers, and a first-class Vault controller API are outside
this POC.

## Credential Model

`credentials_ref` supports either a Kubernetes Secret or a Vault reference:

```json
{
  "credentials_ref": {
    "vault": {
      "path": "postgres/demo",
      "version": 3
    }
  }
}
```

```json
{
  "credentials_ref": {
    "secret": "postgres-credentials"
  }
}
```

Exactly one reference type is required. The Vault reference contains only a
relative path and an optional KV version. Vault address, KV mount,
authentication mount, role, tenant prefix, and TLS configuration are shared
service configuration on the `DataConnectService` resource.

The Vault secret is connector-agnostic. Its fields must match the selected
connection type schema. For PostgreSQL, store the database URL as `URI`; an S3
secret can instead contain `ACCESS_KEY_ID`, `SECRET_ACCESS_KEY`, and `REGION`.

## Design And Security

The authenticated request and stored connection metadata determine the tenant;
users never provide it in a Vault reference. Before resolving credentials, DCH
verifies that the requested connection belongs to the authorized tenant, then
normalizes and combines the configured prefix, tenant, and relative path:

```text
dch/{tenant_id}/postgres/demo
```

Absolute paths, traversal, encoded traversal, empty paths, and paths that can
escape the tenant prefix are rejected. Existing Kubernetes authentication and
authorization, including the Flight service tenant-scoped SubjectAccessReview,
remain the first authorization layer.

For this POC, REST and Flight use service-level Vault roles that can read the
configured `dch/*` hierarchy. DCH enforces tenant isolation through path
construction and authorization, making it the trusted multitenant security
boundary. Production should evaluate tenant-scoped Vault roles and policies,
tenant-specific workload identities, or Vault Enterprise namespaces for
Vault-enforced tenant isolation.

The implementation must:

- Use TLS for every Vault request.
- Never store Vault tokens or resolved credentials in connection metadata.
- Never log service-account JWTs, Vault tokens, secret values, or Vault
  response bodies.
- Reject references containing both source types or neither source type.
- Cache Vault client tokens until shortly before expiry, but never cache secret
  payloads.
- Re-read the projected service-account token during authentication and retry
  authentication once after an authorization failure.
- Reject export requests for Vault-backed connections, preventing externally
  managed credentials from being copied into Kubernetes Secrets.

The PostgreSQL connector cache remains unchanged for this POC. An unversioned
secret rotation can take up to the configured connector cache TTL, currently
about 30 seconds, before a new client resolves the changed value.

## Implementation

The POC changes the shared credential model, API schema, and Python SDK while
preserving persisted Kubernetes Secret references. Connections are stored as
JSON documents, so no metadata database migration is required.

`kube-utils` provides a small Vault client using the existing `reqwest`
dependency. It authenticates with Vault's Kubernetes auth endpoint and reads
KV v2 values. A composite credential resolver routes Kubernetes references to
the existing Secret store and Vault references to this client. Flight uses the
resolver when constructing connector clients; REST uses it for readiness
checks.

The controller propagates `spec.vault` to REST and Flight ConfigMaps and
updates both Deployment pod templates. Each service receives a projected
service-account token at `/var/run/secrets/vault/token` with audience `vault`.
When a custom CA is configured, the controller mounts it at
`/etc/tls/vault/ca.crt`.

## Run The POC

This procedure uses KV v2 static secrets and PostgreSQL. Replace placeholders
with the values for the target cluster.

### 1. Create The Vault Policy And Kubernetes Role

Create a policy scoped to the DCH tenant hierarchy, then bind it to the REST
and Flight service accounts. The policy must use the KV v2 `/data/` path.

```console
vault policy write dch-read - <<'EOF'
path "secret/data/dch/*" {
  capabilities = ["read"]
}
EOF

DCS_NAME=default-dataconnectservice

vault write auth/kubernetes/role/dch \
  bound_service_account_names=dch-rest-service-sa,dch-${DCS_NAME}-flight-sa \
  bound_service_account_namespaces=<namespace> \
  audience=vault \
  policies=dch-read
```

Controller-managed deployments use `dch-rest-service-sa` and
`dch-<DataConnectService-name>-flight-sa`. For the default resource, the
Flight service account is `dch-default-dataconnectservice-flight-sa`. Verify
the rendered names in the target namespace before creating the role.

### 2. Store Connector Credentials

For the default configuration and tenant `opendatahub`, the API reference
`postgres/demo` resolves to `secret/data/dch/opendatahub/postgres/demo`.

```console
vault kv put -mount=secret dch/opendatahub/postgres/demo \
  URI='postgresql://user:password@postgres.example:5432/database?sslmode=require'
```

### 3. Configure Data Connect Hub

Configure `spec.vault` on the `DataConnectService`. This configuration is
shared by every connector, while connector credentials remain only in Vault.

```yaml
apiVersion: dataconnecthub.opendatahub.io/v1alpha1
kind: DataConnectService
metadata:
  name: default-dataconnectservice
  namespace: <namespace>
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

When Vault uses a private CA, create the referenced ConfigMap before applying
the resource:

```console
oc create configmap vault-ca \
  --from-file=ca.crt=vault-ca.crt \
  -n <namespace>
```

When `spec.vault` is omitted, both services continue to use Kubernetes Secret
references. Restart or wait for REST and Flight deployments to roll out after
changing Vault configuration. If a different projected-token audience is used,
change the Vault role and deployment configuration together.

### 4. Create And Audit A Vault-Backed Connection

Create a connection containing only the Vault reference. Replace the connection
type ID with the PostgreSQL connection type installed in the cluster.

```json
{
  "name": "vault-postgres",
  "data_connection_type_id": "<postgres-connection-type-id>",
  "format": "tabular",
  "credentials_ref": {
    "vault": {
      "path": "postgres/demo"
    }
  },
  "properties": {}
}
```

```console
curl --fail-with-body \
  -H 'Content-Type: application/json' \
  -H 'X-Tenant-Id: opendatahub' \
  --data @vault-postgres.json \
  https://<dch-api>/api/v1alpha1/data/connections

curl --fail-with-body -X POST \
  -H 'X-Tenant-Id: opendatahub' \
  https://<dch-api>/api/v1alpha1/data/connections/<connection-id>/readiness
```

Use the normal Flight SQL client with the returned connection ID to execute a
query. The detailed cluster deployment, validation, troubleshooting, and
rollback procedure is in [Vault cluster test](vault-cluster-test.md).

### 5. Validate Rotation And Isolation

Update the Vault `URI`, wait longer than the connector cache TTL, then run the
readiness check and a Flight query again. Confirm that the connection succeeds
without being recreated. Also verify that:

- Vault-backed connections cannot be exported to Kubernetes Secrets.
- A malformed or traversal path is rejected.
- An unauthorized tenant or Vault path fails without exposing sensitive data.
- Existing Kubernetes Secret-backed connections continue to work.

## Validation Coverage

Automated tests cover Kubernetes Secret compatibility, Vault reference
serialization and validation, Kubernetes login requests, KV v2 parsing, token
reuse and re-authentication, tenant path construction, traversal rejection,
missing or malformed Vault values, REST readiness, Flight connector creation,
and sensitive-value redaction.

The POC succeeds when PostgreSQL credentials are resolved directly from Vault,
no Kubernetes Secret contains those credentials, rotations take effect after
the connector cache expires, existing Secret-backed connections keep working,
and Vault access is constrained by tenant and service identity.

## Future Work

Supporting a separate Vault for each tenant requires a provider model rather
than multiple static `[vault]` sections. A tenant-owned provider resource would
contain non-secret configuration such as Vault address, mounts, role or
workload identity, CA bundle reference, and allowed path prefixes. Connections
would reference a provider and relative path:

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

DCH must validate provider ownership, construct paths server-side, and cache
clients and short-lived tokens by tenant and provider. Tokens and resolved
credentials must never be persisted in metadata.

External Vault support also requires DNS, firewall, proxy, and NetworkPolicy
egress; TLS hostname validation and CA rotation; an authentication mechanism
without long-lived static tokens; and, for Kubernetes authentication, Vault
access to the cluster TokenReview API. Kubernetes authentication, OIDC, and
cloud workload identity are candidates depending on Vault location and the
tenant trust model.

Each tenant provider should use a dedicated Vault role and narrowly scoped
policy. Do not grant `list` unless discovery is explicitly needed, and do not
use broad policies such as `secret/data/*`. Vault policies apply to paths, not
individual fields, so credentials requiring separate permissions must be stored
at separate paths. Future design must also address provider onboarding
authorization, role and CA rotation, revocation, audit logging, failure
isolation, and tests preventing a tenant from selecting another tenant's
provider or path.
