# Dev guide: validating changes without a local Rust toolchain

This documents the workflow used to validate Rust/Containerfile changes to
`rest-service` and `flight-service` on machines without a working local
`cargo`/`rustc` (e.g. this was developed entirely on a Mac with no Rust
toolchain installed), and without reliable cross-compilation for `linux/amd64`
(QEMU user-mode emulation on Apple Silicon cannot run this project's `rustc`
version — it segfaults, even on `rustc -vV`).

Instead, both Containerfiles are built and validated directly on the
OpenShift cluster via a `BuildConfig`, which compiles natively on a real
`linux/amd64` node.

## One-time setup

Each service needs a `BuildConfig` pointing at its Containerfile, with a
binary source (so it can build from a local, possibly-uncommitted working
tree — no git push required) and a push secret for the target registry.
These are tracked under `config/openshift-build/{rest-service,flight-service}/`:

```console
oc apply -k config/openshift-build/rest-service -n <your-namespace>
oc apply -k config/openshift-build/flight-service -n <your-namespace>
```

These manifests are **not** ready to use as-is: `output.to.name` and
`output.pushSecret.name` are both literal placeholders
(`<put your image here>`, `<put your push secret here>`) — replace them with
your own registry/tag and a push secret that can actually push there (or use
`kustomize edit set image` for the image). Applying them unedited will fail
with a clear validation error rather than silently doing the wrong thing.

The push secret is a `kubernetes.io/dockerconfigjson` Secret built from local
registry credentials, e.g.:

```console
oc create secret generic <your-push-secret> \
  --from-file=.dockerconfigjson=~/.config/containers/auth.json \
  --type=kubernetes.io/dockerconfigjson \
  -n <your-namespace>
```

## Validating a change

From the repo root, with the fix already made locally (committed or not):

```console
oc start-build rest-service-ubi9 --from-dir=. --follow -n <your-namespace>
oc start-build flight-service-ubi9 --from-dir=. --follow -n <your-namespace>
```

This streams the whole working directory as a build context to the cluster,
runs the Containerfile's `cargo build --release` natively, and pushes the
result on success — surfacing real compile errors exactly like a local
`cargo build` would, just server-side.

The repo's `.dockerignore`/`.containerignore` excludes generated files
from this upload — verified against both `podman build` and
`oc start-build --from-dir`.

If `--follow` disconnects mid-build (transient network blip), the build
itself keeps running — check with `oc get builds -n <your-namespace>` and
re-attach with `oc logs -f bc/<name> -n <your-namespace>`.

## Running the test suite locally

`cargo build --release` on the cluster only proves the code *compiles* —
`#[cfg(test)]` code isn't built by `cargo build` at all. To actually run
`cargo test`, do it natively on your own machine's architecture rather than
cross-compiling (cross-arch `cargo test`/`cargo build` under QEMU emulation
is what segfaults, per above — same-arch is fine and fast):

```console
# Build just the builder stage (source + deps, no cross-arch flag needed —
# let it resolve to your host's native architecture)
podman build --target builder -t dch-rest-builder -f rest-service/Containerfile .

# Run the actual test suite
podman run --rm -w /src dch-rest-builder cargo test --release -p rest-service
```

Two gotchas hit while setting this up, worth knowing about:

- **Stale cached base image architecture**: if you've previously pulled
  `registry.access.redhat.com/ubi9-minimal` for a *different* architecture
  (e.g. via an earlier `--platform linux/amd64` cross-build), podman will
  silently reuse that cached image instead of pulling the native one, and
  you'll hit the same QEMU segfault even without asking for cross-compilation.
  Force the correct platform explicitly if this happens:
  `podman build --platform linux/<your-arch> ...`.
- **VM memory**: compiling the full test binary (arrow, sqlx, actix-web,
  tonic-health, etc. all together) needs more than the 2GB a default
  `podman machine` allocates on macOS — it'll get OOM-killed mid-build with a
  confusing `signal: 9, SIGKILL` error on some arbitrary crate. Bump it:
  `podman machine stop && podman machine set --memory 6144 && podman machine start`.

## End-to-end verification

After a successful build/push, redeploy and check the actual behavior rather
than just "it compiled":

```console
oc apply -k config/base/rest-service -n <your-namespace>
oc apply -k config/base/flight-service -n <your-namespace>
oc rollout status deployment/rest-service -n <your-namespace>
oc rollout status deployment/flight-service -n <your-namespace>

# rest-service: hit a real route
oc exec deploy/rest-service -n <your-namespace> -- curl -s http://127.0.0.1:8080/health

# flight-service: absence of a crash-loop is the signal for anything that
# fails fast on startup (e.g. a bad DB connection)
oc get pods -n <your-namespace> -l app.kubernetes.io/name=flight-service
```

Note: if a Deployment's image tag didn't change (e.g. still `:latest`), a
plain `oc apply -k` won't restart already-running Pods — follow with
`oc rollout restart deployment/<name> -n <your-namespace>` to force a fresh
pull and pick up the new build.

See also: [`docs/user-guide/deploy.md`](../user-guide/deploy.md) for the
full deploy flow this feeds into.
