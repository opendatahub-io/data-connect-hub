# DCH onboarding through the Data module

## Decision and constraints

The platform team confirmed that `Platform.spec.modules.data` represents one
module. DCH will therefore be a capability of the existing Feast module
operator, not a second platform-level module. This supersedes the independent
module design in data-connect-hub#517 and opendatahub-operator#4354.

```text
DataScienceCluster.spec.components.data
  featureStore / dataRegistry / dataConnectHub
                      |
Platform.spec.modules.data + FeastOperator CR
                      |
             feast-module-operator
                /             \
        Feast operator      dc-controller
                                  |
                         DataConnectService
                                  |
                       REST and Flight services
```

Slack guidance is the architectural acceptance criterion:

- One Data module registration and one Platform Data management state.
- The platform installs the module operator and creates its module CR.
- Subconfiguration selects which capabilities the module installs.
- The module reports status on its own CR; the platform writes DSC status.
- DCH stays at `spec.components.data.dataConnectHub`, alongside
  `featureStore` and `dataRegistry` (not `data.feast`).
- Leave Feast's existing unconditional operator installation unchanged, per
  Angad's clarification. A DCH-only configuration may still deploy the Feast
  operator. Removing that deployment is not an acceptance criterion.
- Preserve existing Feature Store, Data Registry, and standalone DCH behavior.
- PostgreSQL and the `dch-database-config` Secret remain administrator-provided
  prerequisites for this release. Database provisioning is out of scope.
- Do not add a `DataConnectHub` module CR, a `dataconnecthub` module handler,
  or a second `Platform.spec.modules` entry.

## Implementation sequence

Current implementation spans the upstream `opendatahub-io/data-connect-hub`,
`opendatahub-io/feast-module-operator`, and `opendatahub-io/opendatahub-operator`
repositories. Feast orchestration is part of upstream onboarding, not downstream
RHOAI delivery. Downstream image provisioning and release-payload integration
remain a separate workstream; the Konflux draft is unpublished preparation.

### 1. DCH application controller

- [x] Retain the namespaced `DataConnectService` API and existing controllers.
- [x] Document the proposed Feast-to-DCH configuration and ownership contract below.
- [x] Verify that the platform version acknowledgment advances only after final
  readiness checks, including advisory checks that can override `Ready`.
- [x] Scope configuration-change reconciliation to services in the changed
  ConfigMap's namespace.
- [x] Run regression tests, the full controller suite, generation, and lint.
- [ ] Rework/supersede #517 without carrying over the independent module CR.

### 2. Feast module operator

- [x] Draft an optional `dataConnectHub` capability, defaulting to Removed for
  existing CRs and existing platform versions.
- [x] Draft DCH gateway and TokenReview audience configuration in the module spec.
  Use the module's existing namespace/platform discovery contract.
- [x] Fetch DCH controller manifests at pinned upstream/downstream revisions.
  Include controller Deployment, CRDs, RBAC, and required networking/metrics
  resources; do not bundle a `DataConnectService` instance.
- [x] Inject controller, REST, Flight, and proxy images from `RELATED_IMAGE_*`.
  Update the module's RBAC for resources it applies, including permission to
  grant the controller's roles. Do not broadly expand permissions.
- [x] Create the owned DCH application CR and runtime configuration.
  Reject conflicting, unowned objects instead of silently adopting them.
- [x] Implement watches for DCH CR status and controller availability. Absent DCH CRDs
  before installation must not prevent the shared module from starting.
- [x] Aggregate DCH health and releases into `FeastOperator.status` only when
  enabled. Do not overwrite the module's `platform` release with a child entry.
- [x] Gate the DCH-enabled Data-module acknowledgment on the child handshake
  and module Deployment availability so all enabled capabilities
  have completed the version handshake.
- [x] On DCH disable, delete its application CR first, wait for its finalizers,
  then remove the DCH controller resources. Retain its CRDs and unrelated data.
- [x] Keep the shared module running while any capability is Managed.
- [x] Apply the same ordered cleanup during complete module removal.
- [x] Unit-test capability combinations, backward compatibility, conflicts,
  readiness, upgrades, disable/re-enable, and teardown ordering.
- [ ] Verify these lifecycle paths with the real controllers on an isolated
  cluster; fake-client tests do not prove Kubernetes GC/finalizer ordering.

### 3. ODH operator

- [x] Implement the public DSC API and v2/v3 conversion coverage, retaining the
  public shape proposed in #4354 but not its independent module architecture.
- [x] Project `dataConnectHub` into the existing `FeastOperator` module CR.
- [x] Enable `Platform.modules.data` when Feature Store OR Data Registry OR
  DCH is Managed. Omitted DCH configuration must mean Removed.
- [x] Extend the existing Data handler's related images, not a new handler.
- [x] Keep the existing Data status writer; do not carry over the proposed two-module
  `DataReady` aggregation and independent module registration.
- [ ] Pin the Feast module manifests/image containing the new capability.
- [x] Generate and validate DSC schemas and API conversions for all served
  versions. Verify DCH fields survive a v2/v3 round trip.
- [x] Exercise the same eight capability combinations at handler level.

### 4. RHOAI controller-image onboarding (required for either architecture)

Working release assumption: `rhoai-3.7-ea.1`, as referenced in #4354.
Release engineering must confirm the target before publication.

- [ ] Synchronize required DCH changes into the downstream source release branch.
- [ ] Onboard a Konflux controller Component with build/release engineering.
  Pipeline YAML alone does not provision its service account, credentials,
  registry repository, release configuration, or product approval.
- [x] Prepare and structurally validate a local draft controller push PipelineRun
  in `red-hat-data-services/konflux-central` on the target release base.
- [ ] Submit the controller PipelineRuns in `red-hat-data-services/konflux-central`,
  under `pipelineruns/data-connect-hub/.tekton/`, on the target release branch.
  Do not edit synchronized `.tekton/` files in the DCH repository.
- [ ] Build from repository root using `dc-controller/Containerfile.konflux`;
  prefetch Go dependencies from `dc-controller/`, not the repository root.
- [ ] Verify hermetic build compatibility, FIPS, architecture support, scans,
  provenance, and release policy with the build team. Do not copy the REST
  image's Rust/FIPS exception to the Go controller.
- [ ] Publish a controller digest. Proposed image repository:
  `quay.io/rhoai/odh-data-connect-hub-controller-rhel9`.
- [ ] Add `RELATED_IMAGE_ODH_DATA_CONNECT_HUB_CONTROLLER_IMAGE` to the target
  `RHOAI-Build-Config` payload and image repository mappings, provenance
  metadata, and image-update/nudging configuration through release automation.
- [ ] Confirm REST/Flight/proxy images remain present in the same target release.
  REST and Flight were offboarded from 3.6; do not assume all branches match.
- [ ] Update supported Helm payloads if required by the selected delivery scope.
- [ ] Synchronize the Feast module and platform changes into their downstream
  release branches and pin matching source/image revisions.
- [ ] Remove the temporary controller-image exception from #4354 once the
  downstream payload actually contains the image.
- [ ] Verify the final CSV/payload contains the controller image and its env
  variable reaches Feast, then dc-controller's Deployment image. Verify operand
  image env variables reach dc-controller. Test disconnected mirroring.

## Proposed Feast-to-DCH contract

Use existing application APIs rather than introducing another module boundary.
The details below are implementation proposals for maintainer review, not an
already published Feast API.

| Concern | Contract |
| --- | --- |
| Module owner | Existing cluster-scoped `FeastOperator` singleton |
| Application | `DataConnectService/default-dataconnectservice` in `odh-data-connect-hub-infra` (ODH) or `rhoai-data-connect-hub-infra` (RHOAI), overridable through the DSC |
| Owner reference | Application CR controlled by `FeastOperator`; DCH operands remain owned by the application CR |
| User/platform configuration | Gateway and TokenReview audiences projected into the application CR spec |
| Runtime metadata | Feast owns `opendatahub-dataconnecthub-config` in the DCH infrastructure namespace, containing distribution metadata and the forwarded `platformVersion` from `odh-data-config` |
| Gateway routing | HTTPRoute stays in the applications namespace via `spec.gateway.routeNamespace`; named-Service ReferenceGrants live with the DCH backends |
| Legacy connection migration | Original tenant Secrets remain in place; migration discovers REST from the DCH CR's infrastructure namespace and stores Secret references, not credential copies |
| Existing config conflict | Refuse unowned/conflicting application CRs and ConfigMaps; define explicit adoption separately if required |
| Standalone compatibility | Existing ConfigMap name and application API continue to work without Feast |
| Application overrides | Patch only platform-owned spec fields; preserve REST/Flight sizing, tracing, connectors, and other application overrides |
| Readiness | Current child observedGeneration, Ready=True, provisioning success, and controller available; propagate degradation |
| Upgrade acknowledgment | Preserve the previous platform release until all enabled capabilities finish upgrading |
| DCH removal | Delete application, wait for finalizers, remove controller resources; do not remove the shared module |

The existing DCH ConfigMap is an application-internal adapter. It is not a
second platform-managed module handshake. Platform/user gateway and auth inputs
must flow through the module CR and application CR specs, not be exposed as new
platform-managed ConfigMap APIs.

## Validation matrix and review gates

At each boundary, review the diff against the constraints above before moving
on. Do not declare onboarding complete based only on unit tests.

1. All eight combinations of Feature Store, Data Registry, and DCH:
   the shared module is enabled iff at least one capability is Managed.
2. Legacy Feast CR without the DCH capability: no DCH resources created.
3. DCH-only: application works; existing Feast operator installation is allowed.
4. Mixed install: existing capabilities retain behavior; DCH health contributes
   to Data status without separate platform-level status writers.
5. DCH disable/re-enable while Feast remains enabled: ordered cleanup and
   no deletion of unrelated resources or application CRDs.
6. Full Data removal: module finalization waits for DCH cleanup while
   dc-controller is still available.
7. Stale status, missing database Secret, unavailable controller, failed
   provisioning, degradation, and pending upgrade all prevent false readiness.
8. Existing unowned application/config: explicit conflict, not adoption.
9. Config updates: spec fields propagate, application overrides survive,
   namespace-specific events do not enqueue unrelated applications.
10. RHOAI image build, payload inclusion, image propagation, fresh install,
    upgrade, disable/re-enable, and disconnected smoke test.

## Open prerequisites

- Confirm the RHOAI target release and obtain the controller's Konflux component,
  release, registry, and architecture approvals from build/release engineering.
- Agree with Feast maintainers on the API shape and application ownership.
- The administrator must provision PostgreSQL and `dch-database-config` in the
  application's namespace before DCH can become Ready. Neither dc-controller
  nor the planned module integration provisions the database in this release.
- Decide whether existing standalone DCH deployments require an explicit
  migration/adoption procedure; refusal to adopt is the safe initial behavior.

## Implementation and validation checkpoints

These source changes do not yet constitute completed cluster onboarding.

| Repository | Changes |
| --- | --- |
| data-connect-hub | Architecture guidance, application version-handshake regression fix, namespace-scoped configuration watch, regression tests |
| feast-module-operator | Optional DCH capability, pinned application-controller manifests, ownership-safe install/cleanup, deferred API watch, readiness/releases/upgrade aggregation, generated RBAC/chart, lifecycle tests |
| opendatahub-operator | Existing Data-handler projection/enablement/image wiring, public DSC API, v2/v3 conversion, generated API docs/schemas, sample updates, tests; no new Platform module |
| konflux-central | Unpublished draft controller push PipelineRun using Go prefetch at `dc-controller/`; target release and provisioning require confirmation |

Validation:

- DCH `make test`: passed, including envtest and application manifest tests.
- DCH `make lint-fix` and `make lint`: passed after fixing a test-style issue.
- DCH envtest additionally verifies that a module SSA field manager can apply and
  clear gateway/auth settings while preserving administrator sizing/tracing.
- Feast `make test`, `make lint`, and `make verify-rbac`: passed, including
  backward compatibility, all eight capability enablement combinations, generated
  CRD schema validation, actual bundle rendering/image injection, application
  conflicts, finalizer waits, current-generation status, and upgrade gating.
  Schema-validation tests required corresponding existing-version Kubernetes
  validation dependencies with `go mod tidy`; no dependency versions were changed.
- ODH Data-handler, builtin-registry, and DSC v2 conversion tests: passed.
  `make generate manifests api-docs`, `make manifests ODH_PLATFORM_TYPE=rhoai`,
  `make fmt lint`, and `make build`: passed. `make bundle-all` validated both ODH
  and RHOAI bundles. The scoped v3 webhook/envtest suite passed, including DCH
  overrides surviving API-server admission and rejection of an empty Gateway
  namespace. These are local API-server tests, not live operand deployment tests.
- Konflux upstream validator: 11 checks passed, one Quay repository existence
  check skipped because registry credentials were unavailable. No controller
  Component/service account has been provisioned and no image has been built.
- Chart generation exposed unrelated pre-existing chart customizations; retained
  existing image defaults, network policy naming, and upgrade-gate template.

### Remaining publication and deployment gates

- Review the linked upstream draft PRs. The previous independent-module PRs are
  superseded by this architecture and must not be merged as-is.
- Refresh the Feast DCH-manifest pin after the DCH controller changes are merged;
  the current manifest-only pin is an existing published revision, not a claim
  that it contains this branch's controller changes.
- Publish the updated Feast module image/chart and pin their matching revisions
  in the ODH operator. Do not invent a remote SHA or image digest for local,
  uncommitted code. The existing ODH manifest/image pins do not yet contain this
  implementation, so the generated ODH payload is not an end-to-end DCH install.
- Run isolated-cluster installation/upgrade/removal tests, including the module's
  deferred custom-API watch and real garbage collection.
- Complete downstream image provisioning and release-payload integration using
  [the Konflux handoff](rhoai-controller-image-onboarding.md).

## References

- Platform module contract: `opendatahub-operator/internal/controller/modules/`.
- Module Handler Developer Guide supplied by the platform team.
- Existing PRs: data-connect-hub#517 and opendatahub-operator#4354.
- Pipeline ownership: `red-hat-data-services/konflux-central/README.md`.
- RHOAI image onboarding precedent: RHOAI-Build-Config#30670.
