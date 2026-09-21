# Vault cluster test

This runbook validates Vault-backed credentials on the development cluster.
It assumes the PR's controller, REST, and Flight images have already been
built and published by Konflux.

## Variables

```console
export DCH_NS=redhat-ods-applications
export VAULT_NS=vault-test
export DCS=default-dataconnectservice

export VAULT_ADDR=https://vault-internal.vault-test.svc:8200
export VAULT_ROLE=dch
export VAULT_MOUNT=secret
export VAULT_PREFIX=dch

export REST_SA=dch-rest-service-sa
export FLIGHT_SA=dch-default-dataconnectservice-flight-sa
```

Use `vault-internal.vault-test.svc`, not `vault.vault-test.svc`. The Vault
certificate only includes the internal service DNS names.

## Verify Vault

Vault must be initialized and unsealed:

```console
oc exec -n "$VAULT_NS" vault-0 -- \
  env VAULT_ADDR="$VAULT_ADDR" VAULT_SKIP_VERIFY=true vault status -format=json
```

From the existing REST Pod, verify that Vault is reachable. `--insecure` is
only used for this pre-deployment network check because the stable deployment
does not yet mount the Vault CA:

```console
oc exec -n "$DCH_NS" deploy/dch-rest-service -c rest-service -- \
  curl --silent --show-error --fail --insecure \
  "$VAULT_ADDR/v1/sys/health?standbyok=true"
```



## Deploy The PR Images

Apply the generated CRD before using `spec.vault`:

```console
oc apply -f dc-controller/config/crd/bases/dataconnecthub.opendatahub.io_dataconnectservices.yaml
```

Set the Konflux image references produced for the PR:

```console
export CONTROLLER_IMAGE=quay.io/opendatahub/odh-kube-rbac-proxy:odh-pr
export REST_IMAGE=quay.io/opendatahub/odh-data-connect-hub-rest:odh-pr
export FLIGHT_IMAGE=quay.io/opendatahub/odh-data-connect-hub-flight:odh-pr

oc set image deployment/dc-controller-manager \
  manager="$CONTROLLER_IMAGE" \
  -n dc-controller-system

oc set env deployment/dc-controller-manager \
  RELATED_IMAGE_ODH_DATA_CONNECT_HUB_REST_IMAGE="$REST_IMAGE" \
  RELATED_IMAGE_ODH_DATA_CONNECT_HUB_FLIGHT_IMAGE="$FLIGHT_IMAGE" \
  -n dc-controller-system

oc rollout status deployment/dc-controller-manager -n dc-controller-system
```



## Configure Vault Authentication

Open an interactive Vault shell and enter a Vault administrator token there.
Do not put the token in shell history or a manifest.

```console
oc rsh -n "$VAULT_NS" vault-0
```

Run the following inside the Vault Pod:

```console
export VAULT_ADDR=https://vault-internal.vault-test.svc:8200
export VAULT_SKIP_VERIFY=true
export VAULT_TOKEN='<Vault administrator token>'

vault auth list -format=table
vault secrets list -detailed
```

Enable Kubernetes authentication only if `kubernetes/` is absent:

```console
vault auth enable kubernetes
```

Configure it using the Vault service account. The cluster already grants this
service account `system:auth-delegator`.

```console
export TOKEN_REVIEW_JWT="$(cat /var/run/secrets/kubernetes.io/serviceaccount/token)"
export KUBERNETES_CA_CERT="$(cat /var/run/secrets/kubernetes.io/serviceaccount/ca.crt)"

vault write auth/kubernetes/config \
  token_reviewer_jwt="$TOKEN_REVIEW_JWT" \
  kubernetes_host=https://kubernetes.default.svc \
  kubernetes_ca_cert="$KUBERNETES_CA_CERT"
```

Enable a KV v2 engine only if the `secret/` mount is absent:

```console
vault secrets enable -path="$VAULT_MOUNT" -version=2 kv
```

Create a policy and a role bound to the rendered DCH service accounts:

```console
vault policy write dch-read - <<EOF
path "${VAULT_MOUNT}/data/${VAULT_PREFIX}/${DCH_NS}/*" {
  capabilities = ["read"]
}
EOF

vault write auth/kubernetes/role/"$VAULT_ROLE" \
  bound_service_account_names="$REST_SA,$FLIGHT_SA" \
  bound_service_account_namespaces="$DCH_NS" \
  audience=vault \
  policies=dch-read \
  ttl=15m

vault read auth/kubernetes/role/"$VAULT_ROLE"
vault policy read dch-read
```



## Store Test Credentials

For PostgreSQL, store the `URI` field expected by the PostgreSQL connection
type. Substitute a valid test database URI:

```console
vault kv put -mount="$VAULT_MOUNT" \
  "$VAULT_PREFIX/$DCH_NS/postgres/demo" \
  URI='postgresql://testuser:testpassword@dch-postgres:5432/testdb?sslmode=disable'
```

For another connector, use a distinct path and the fields from its connection
type. For example, an S3 document can contain `ACCESS_KEY_ID`,
`SECRET_ACCESS_KEY`, and `REGION`.

## Configure DCH

The existing `openshift-service-ca.crt` ConfigMap in the DCH namespace trusts
the OpenShift service-serving signer used by Vault. Configure the DCH CR to
mount it and generate the service `[vault]` configuration:

```console
oc patch dataconnectservice "$DCS" \
  -n "$DCH_NS" \
  --type=merge \
  --patch "$(cat <<EOF
spec:
  vault:
    address: ${VAULT_ADDR}
    role: ${VAULT_ROLE}
    kvMount: ${VAULT_MOUNT}
    authMount: kubernetes
    tenantPrefix: ${VAULT_PREFIX}
    caConfigMap:
      name: openshift-service-ca.crt
      key: service-ca.crt
EOF
)"
```

Wait for reconciliation and confirm the rendered state:

```console
oc rollout status deployment/dch-rest-service -n "$DCH_NS"
oc rollout status deployment/dch-default-dataconnectservice-flight -n "$DCH_NS"

oc get configmap dch-rest-service-config -n "$DCH_NS" \
  -o jsonpath='{.data.config\.toml}' | rg -A8 '^\[vault\]'

oc get configmap dch-default-dataconnectservice-flight-config -n "$DCH_NS" \
  -o jsonpath='{.data.config\.toml}' | rg -A8 '^\[vault\]'
```

Both Deployments must have a projected service-account token with audience
`vault`, and both ConfigMaps must contain the `[vault]` section.

## Create And Audit A Connection

Port-forward the REST service:

```console
oc port-forward -n "$DCH_NS" svc/dch-rest-service 8443:8443
```

In another terminal, use a cluster token to find the PostgreSQL connection
type ID:

```console
export API=https://localhost:8443
export TOKEN="$(oc whoami -t)"

curl --silent --insecure \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Tenant-Id: $DCH_NS" \
  "$API/api/v1alpha1/data/connection-types" | jq .
```

Create `vault-postgres.json`, replacing the connection type ID:

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

Create and audit the connection:

```console
CONNECTION_ID="$(
  curl --silent --show-error --fail --insecure \
    -H "Authorization: Bearer $TOKEN" \
    -H 'Content-Type: application/json' \
    -H "X-Tenant-Id: $DCH_NS" \
    --data @vault-postgres.json \
    "$API/api/v1alpha1/data/connections" |
  jq -r '.metadata.id'
)"

curl --silent --show-error --fail --insecure -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Tenant-Id: $DCH_NS" \
  "$API/api/v1alpha1/data/connections/$CONNECTION_ID/readiness"
```

The readiness endpoint must return HTTP `204`.

## Rotation And Negative Tests

Update the Vault value, wait longer than the 30-second PostgreSQL pool cache
TTL, then rerun readiness and a normal Flight query:

```console
vault kv put -mount="$VAULT_MOUNT" \
  "$VAULT_PREFIX/$DCH_NS/postgres/demo" \
  URI='postgresql://<rotated-user>:<rotated-password>@dch-postgres:5432/<database>?sslmode=disable'

sleep 35

curl --silent --show-error --fail --insecure -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Tenant-Id: $DCH_NS" \
  "$API/api/v1alpha1/data/connections/$CONNECTION_ID/readiness"
```

Confirm Vault-backed exports fail:

```console
curl --silent --show-error --insecure -X PUT \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Tenant-Id: $DCH_NS" \
  "$API/api/v1alpha1/data/connections/$CONNECTION_ID/exports/secrets/should-fail"
```

Confirm unsafe Vault paths are rejected:

```console
curl --silent --show-error --insecure \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -H "X-Tenant-Id: $DCH_NS" \
  --data '{"name":"bad-vault-path","data_connection_type_id":"<postgres-connection-type-id>","format":"tabular","credentials_ref":{"vault":{"path":"../other-tenant"}},"properties":{}}' \
  "$API/api/v1alpha1/data/connections"
```

## Troubleshooting

### Variables Are Empty In The Vault Pod

Variables exported in the local terminal are not inherited by `oc rsh`. Set
them again inside the Vault Pod:

```console
export VAULT_ADDR=https://vault-internal.vault-test.svc:8200
export VAULT_SKIP_VERIFY=true
export VAULT_MOUNT=secret
export VAULT_PREFIX=dch
export DCH_NS=redhat-ods-applications
export VAULT_ROLE=dch
export REST_SA=dch-rest-service-sa
export FLIGHT_SA=dch-default-dataconnectservice-flight-sa
```

### Vault Uses 127.0.0.1 And TLS Fails

The Vault Pod commonly sets `VAULT_ADDR` to `https://127.0.0.1:8200`, but the
serving certificate does not contain `127.0.0.1`. Use the internal Service DNS
name instead:

```console
export VAULT_ADDR=https://vault-internal.vault-test.svc:8200
export VAULT_SKIP_VERIFY=true
vault status
```

`VAULT_SKIP_VERIFY=true` is for the administrative Vault shell only. DCH uses
the mounted OpenShift service CA and verifies Vault TLS normally.

### Vault Returns 403 When Writing A Secret

The DCH `dch-read` policy cannot write secrets and may not perform Vault CLI
mount preflight checks. Use the Vault `root_token` or another administrator
token for setup commands, entered interactively:

```console
read -s VAULT_TOKEN
export VAULT_TOKEN
vault token lookup
```

The setup token should show the `root` policy or an equivalent administrative
policy. Never put this token in a manifest or give it to DCH. DCH authenticates
through Kubernetes auth and uses only the limited read policy.

### Vault Returns 404 For A Secret

DCH constructs the path from the authenticated tenant and the relative
connection reference:

```text
dch/<tenant-id>/<credentials_ref.vault.path>
```

For a connection with tenant `vish-test` and path `postgres/demo`, verify:

```console
vault kv get -mount=secret dch/vish-test/postgres/demo
```

The policy must also permit that tenant path:

```console
vault policy write dch-read - <<EOF
path "secret/data/dch/vish-test/*" {
  capabilities = ["read"]
}
EOF
```

### PostgreSQL Credentials Fail

The PostgreSQL URI must use the PostgreSQL Service in the tenant namespace.
For the test database used by this runbook, the Service is:

```text
postgres.vish-test.svc.cluster.local:5432
```

The deployment imports its credentials from the `postgres-auth` Secret:

```console
oc get secret postgres-auth -n vish-test -o json | jq -r '.data | keys[]'
```

Use the actual `POSTGRES_USER`, `POSTGRES_PASSWORD`, and `POSTGRES_DB` values
when updating Vault. Do not assume placeholder values such as `testuser`,
`testpassword`, or `testdb` are valid:

```console
vault kv put -mount=secret \
  dch/vish-test/postgres/demo \
  URI='postgresql://<user>:<password>@postgres.vish-test.svc.cluster.local:5432/<database>?sslmode=disable'
```

### Directly Test Vault From Flight

This checks Kubernetes authentication and the exact KV path without exposing
the service-account JWT or Vault token. Run it from the Flight Pod:

```console
oc exec -n redhat-ods-applications \
  deploy/dch-default-dataconnectservice-flight -- sh -c '
set -eu
base=https://vault-internal.vault-test.svc:8200
jwt=$(cat /var/run/secrets/vault/token)
login=$(curl --silent --show-error --fail \
  --cacert /etc/tls/vault/ca.crt \
  -X POST "$base/v1/auth/kubernetes/login" \
  -H "Content-Type: application/json" \
  --data "{\"role\":\"dch\",\"jwt\":\"$jwt\"}")
vault_token=$(printf "%s" "$login" | sed -n "s/.*\"client_token\":\"\([^\"]*\)\".*/\1/p")
test -n "$vault_token"
status=$(curl --silent --output /dev/null --write-out "%{http_code}" \
  --cacert /etc/tls/vault/ca.crt \
  -H "X-Vault-Token: $vault_token" \
  "$base/v1/secret/data/dch/vish-test/postgres/demo")
'
```

Expected result after the secret and policy are correct:

```text
login=ok secret_http_status=200
```
