# Deploying Data Connect Hub (current state)

This documents the current, working way to deploy Data Connect Hub to an
OpenShift cluster using the Kustomize manifests under `config/`. It reflects
what's actually been stood up and tested so far, not a finished/final
deployment story.

## Layout

```text
config/
  base/
    rest-service/     # HTTP API (actix-web), port 8080
    flight-service/    # Arrow Flight gRPC service, port 50051
  db/
    postgres/          # Postgres instance backing both services
```

Each directory is a self-contained Kustomization, applied independently.
There is no top-level `config/base/kustomization.yaml` yet aggregating all
three — apply them one at a time as shown below.

## Prerequisites

- OpenShift 4.20+ (Kubernetes 1.33+) — required for the native `grpc:`
  readiness/liveness probe type used by `flight-service`. Validated against
  a 4.20.31 cluster.
- Logged in to the target cluster (`oc login` / `oc whoami` should work)
- A namespace to deploy into. These docs use `redhat-ods-applications`,
  which is what we validated against — swap it for your own namespace.
- `openssl` available locally (used by the secret-generation script)

### Image pulls

- `rest-service` / `flight-service` images live at
  `ghcr.io/opendatahub-io/data-connect-hub/{rest-service,flight-service}:latest`,
  built by this repo's CI, with `imagePullPolicy: Always`. This is
  intentional for now, while the project is early and changing fast —
  expect this to move to pinned, intentionally-updated digests as things
  stabilize. **Note:** `imagePullPolicy: Always` only affects Pods when they
  start — it does **not** make `oc apply -k` refresh already-running Pods
  after a new image is pushed to `:latest`. Force that with:
  ```console
  oc rollout restart deployment/rest-service -n <your-namespace>
  oc rollout restart deployment/flight-service -n <your-namespace>
  ```
- `registry.redhat.io/rhel9/postgresql-16` requires registry auth, but on
  most OpenShift clusters the cluster-wide pull secret already covers
  `registry.redhat.io` — no extra pull secret is needed for Postgres. If
  your cluster doesn't have that configured, you'll need to add one.
- `rest-service`/`flight-service` each have their own `ServiceAccount`
  (`data-connect-hub-sa`, `flight-service-sa`) referencing an
  `imagePullSecrets` entry named `dch-pull-secret`. That secret must already
  exist in your target namespace and cover whichever registry actually
  hosts your images.

## 1. Generate Postgres credentials (once per environment)

Postgres credentials are **not** committed to the repo. Kustomize's
`secretGenerator` builds the real `Secret` from two local files that you
generate once per environment and that are git-ignored:

```console
./config/db/postgres/generate-secrets.sh
```

This creates, next to the script (both `chmod 600`, owner-read/write only):
- `postgres-credentials.env` — `POSTGRESQL_USER` / `POSTGRESQL_PASSWORD` /
  `POSTGRESQL_DATABASE`, consumed by the Postgres container itself.
- `secret-config.toml` — a `[database]` TOML snippet with the full connection
  URL, mounted into `rest-service`/`flight-service` as a file (see below).

The script refuses to overwrite existing files — delete them first if you
genuinely want to rotate credentials (see the note on rotation below).

`*.env.example` / `*.toml.example` in the same directory show the expected
shape if you want to hand-author these instead.

## 2. Deploy Postgres

```console
oc apply -k config/db/postgres -n <your-namespace>
oc rollout status deployment/postgres -n <your-namespace>
```

This creates a `Secret` (from the generated files), a 5Gi `PersistentVolumeClaim`
(default StorageClass), a `Deployment` (single replica, `Recreate` strategy —
required since the PVC is `ReadWriteOnce`), a `Service` reachable
in-namespace at `postgres:5432`, and a `NetworkPolicy` (currently allow-all
ingress/egress — a placeholder to tighten once we have a defined
gateway/client topology, not a real restriction yet).

**Credential rotation caveat:** this Postgres image only sets a role's
password at first `initdb`; on every subsequent start it runs
`ALTER ROLE <user> PASSWORD ...` for whatever's in the Secret. That means you
can safely rotate the **password** by deleting *both* `postgres-credentials.env`
and `secret-config.toml`, rerunning `generate-secrets.sh`, re-applying, and
restarting Postgres — but you **cannot** change `POSTGRESQL_USER` without
wiping `postgres-data` and reinitializing, since the role won't exist yet
under the new name.

Note the Secret's name is intentionally kept stable (no content-hash
suffix — see the comment in `config/db/postgres/kustomization.yaml`), since
`rest-service`/`flight-service` reference it by static name from separate
Kustomize trees. The tradeoff: rotating it does **not** automatically roll
those two Deployments — you must restart them manually (next section).

## 3. Deploy rest-service and flight-service

```console
oc apply -k config/base/rest-service -n <your-namespace>
oc apply -k config/base/flight-service -n <your-namespace>
oc rollout status deployment/rest-service -n <your-namespace>
oc rollout status deployment/flight-service -n <your-namespace>
```

Both services get their `config.toml` from a `ConfigMap` (server/cache
settings only — no credentials) and their database connection info from
`/secrets/secret-config.toml`, mounted from the same `postgres-credentials`
Secret Postgres uses. The app merges both file sources at startup, with the
mounted secret file taking priority for the `[database]` section.

If you rotate the Postgres password (see above), restart **both** of these
deployments afterward so they re-read the updated secret file — mounted
Secret volumes update automatically, but the app only reads config once at
startup:

```console
oc rollout restart deployment/rest-service -n <your-namespace>
oc rollout restart deployment/flight-service -n <your-namespace>
```

## 4. Verify

```console
# rest-service has a real health route now
oc exec deploy/rest-service -n <your-namespace> -- curl -s http://127.0.0.1:8080/health

# and the app can actually query the DB
oc exec deploy/rest-service -n <your-namespace> -- \
  curl -s http://127.0.0.1:8080/v1/data/connections

# flight-service: Ready 1/1 means its gRPC health check (tonic-health) is
# passing, which only happens after it successfully connects to Postgres
oc get pods -n <your-namespace> -l app.kubernetes.io/name=flight-service
```

## Known gaps / follow-ups

- No top-level Kustomization ties `rest-service` + `flight-service` +
  `postgres` together yet; each is applied separately. This also means
  Postgres secret rotation can't be auto-propagated to its consumers (see
  above) without merging into one kustomization.
- No `odh` / `rhoai` overlays yet (`config/overlays/` exists but is empty).
- `NetworkPolicy` resources exist for all three components but currently
  allow all ingress/egress — real restriction is pending a defined
  gateway/client topology.
- Postgres has no backup/restore story — it's a single instance with a
  Kubernetes-managed PVC, fine for dev but not a substitute for real backups.
