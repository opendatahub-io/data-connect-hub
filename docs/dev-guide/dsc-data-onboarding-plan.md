# Data Connect Hub onboarding under DSC Data

- Status: implementation handoff for Luna
- Scope: ODH and RHOAI integration through `spec.components.data`

## Start here: agreed direction

**DCH is an independent platform module, alongside Feast.** Its public switch is
`spec.components.data.dataConnectHub.managementState` (working field name).
The ODH/RHOAI platform operator installs DCH and creates its module CR directly,
through a dedicated DCH handler. DCH's module controller then manages the
namespaced `DataConnectService` and reports its status.

- The public `data` object groups capabilities; it does not establish Kubernetes
  ownership between their operators.
- DCH installation, upgrade, removal, and reconciliation must work without Feast.
- The existing Feast handler continues to manage Feature Store and Data Registry.
- A separate Data orchestrator repository or a Feast-to-DCH ownership chain is
  not part of this design.
- Main implementation PRs belong in `opendatahub-io/data-connect-hub` and
  `opendatahub-io/opendatahub-operator`. Follow the release process for downstream
  synchronization and any release-specific follow-ups.

### Local handoff state

Both working directories are under `/Users/angsingh/Desktop/redhat/`:

| Checkout | Branch | Handoff state |
| --- | --- | --- |
| `opendatahub-operator` | `feat/dsc-data-connect-hub` | Uncommitted API/conversion foundation plus independent DCH platform handler, enablement, readiness/release aggregation, and RBAC changes. |
| `data-connect-hub` | `main` | This plan and the DCH module API/controller, packaging, RBAC, and tests are uncommitted. Existing untracked `artifacts/` is unrelated work. |
| `rhods-operator` | Research checkout | Consult for downstream compatibility; no implementation changes made. |
| `feast-module-operator` | Research checkout | Reference implementation only; no implementation changes made. |

The parent implementation is **local and uncommitted**. Switching to the branch
on another machine will not retrieve it. Transfer or commit these files when
handing work off outside this workspace:

```text
api/components/v1alpha1/data_types.go
api/components/v1alpha1/zz_generated.deepcopy.go
api/datasciencecluster/v2/datasciencecluster_conversion.go
api/datasciencecluster/v2/datasciencecluster_conversion_test.go
api/datasciencecluster/v2/dataconnecthub_conversion_test.go  # new, untracked
docs/api-overview.md
```

Read each repository's `AGENTS.md` and contribution instructions before editing.
For DCH API scaffolding, inspect `dc-controller/PROJECT`: at handoff it records a
`DataConnectHub` resource under a path that does not match the current namespaced
application API layout. Resolve scaffolding metadata through Kubebuilder rather
than overwriting existing APIs or hand-editing generated metadata.

### Recommended implementation and PR order

1. Agreed for implementation: public DSC field `dataConnectHub`, module API
   `DataConnectHub`, singleton `default-dataconnecthub`, internal key
   `dataconnecthub`; administrator overrides include gateway and
   TokenReview audiences.
2. Implement and test the DCH module controller and bootstrap packaging in
   `data-connect-hub`. Its existing application controller remains responsible
   for REST/Flight resources. Choose separate module/operand entrypoints or a
   justified combined process following the onboarding guide.
3. Complete the parent branch with the DCH handler, Platform API projection,
   manifest/image registration, public Data status aggregation, and lifecycle
   tests. Keep this PR dependent on the DCH implementation and pin an available
   module manifest revision/image before merge.
4. Validate the combined installation and upgrade in an isolated environment,
   then follow upstream/downstream delivery procedures for the chosen release.

Maintainer answers (2026-11-22): use RHOAI ticket RHOAIENG-79055 for the main
onboarding and RHOAIENG-95346 for the CRD update; target upstream ODH `main` and
RHOAI `rhoai-3.7-ea.1`; refuse adoption of unowned DCS instances; support
applications-namespace changes through ordered cleanup/recreation; and expose
gateway and TokenReview-audience overrides in the DSC. The ODH branch
`odh-3.7-ea.1` does not exist on the checked upstream remote. ODH `main` and
`stable` currently resolve to the same commit.

As of 2026-11-22, upstream refs were ODH `main` at
`b68a6fb48734ef7d2de91a1383c34bf68c47499a`, upstream DCH `main` at
`b08cdc82e7ec5b3141bf6691839b14eb6087abc6`, RHOAI operator
`rhoai-3.7-ea.1` at `2945611892963725bcc5312d21fc646975449297`, and downstream
DCH `rhoai-3.7-ea.1` at `66968dd907fd924002c033cfff208ccdd165d17a`. The DCH module
implementation is not yet in those refs, so do not use them as the final module
manifest pin. The RHOAI 3.7 manifest configuration currently has DCH REST/Flight
pins but no DCH controller image pin.

## Implementation progress and PR map

Work started on 2026-11-18. The first development branch is
`feat/dsc-data-connect-hub` in the sibling checkout `../opendatahub-operator`,
based on upstream commit `b68a6fb48734ef7d2de91a1383c34bf68c47499a`.
These are local changes; no PR has been published and no cluster changes have
been made.

Completed checks for the API foundation:

- `make generate manifests api-docs`.
- `make manifests ODH_PLATFORM_TYPE=rhoai`.
- `go test ./api/datasciencecluster/v2 ./api/datasciencecluster/v3`.
- `make fmt lint` (zero issues).
- Inspection of both generated DSC schemas: DCH is optional in v3, accepts
  `Managed`/`Removed`, and does not alter the v2 public shape.

Runtime and cluster integration validation remain outstanding.

### Implementation handoff update (2026-11-22)

Local implementation now follows the intended DCH-first, parent-second order:

1. **`data-connect-hub`**: added the approved cluster-scoped singleton
   `DataConnectHub` API, reconciliation of the namespaced
   `default-dataconnectservice`, status/release projection, ownership-conflict
   rejection, and ordered finalizer cleanup. The existing DataConnectService
   API remains namespaced. Both controllers share the existing manager binary;
   this keeps the application reconciler running while module finalizers drain.
   Added the module CRD to the Kustomize and Helm bundles, RBAC, samples, and
   corrected the root API-scope guidance. The DCH controller watches and merges
   the platform-owned `odh-dataconnecthub-config` only for its owner-referenced
   application CR; independently managed services continue to use the legacy
   standalone ConfigMap/defaults and are not affected by DCH module settings.
2. **`opendatahub-operator`**: added a sibling `dataconnecthub` handler and
   independent Platform enablement, projects applications namespace,
   distribution, default or explicitly overridden gateway and TokenReview
   audiences, preserves these public overrides through v2 reads/writes, and
   aggregates public `DataReady`/`status.components.data` once across Feast and
   DCH. The Feast handler continues to depend only on Feature Store and Data
   Registry. Added platform-operator RBAC for the new module CR.

The public API/conversion foundation and current local parent/runtime tests are
still uncommitted on `feat/dsc-data-connect-hub`. The DCH implementation is also
uncommitted. No PRs or live cluster changes have been made.

**Not merge-ready:** for the ODH-first parent change, `manifests-config.yaml`
still needs the DCH module source and updated ODH controller image pinned to the
post-implementation DCH revision/build. The parent module handler is deliberately
not wired to an older DCH manifest snapshot. RHOAI controller image/source
registration remains a downstream follow-up; the image base
`quay.io/rhoai/odh-data-connect-hub-controller-rhel9` is agreed, but its digest
must come from that build. The Kubebuilder `PROJECT` row also retains a stale
`namespaced: true` flag even though generated DCH CRD scope is Cluster;
Kubebuilder preserved the existing row when invoked with `--namespaced=false`.
The maintainer requested deferring this metadata discrepancy until an approved
Kubebuilder workflow is available; do not hand-edit generated metadata.

### Remaining maintainer follow-ups

- Publish/pin the post-implementation DCH manifest revision and ODH controller
  image digest, then obtain the agreed RHOAI controller image digest through the
  downstream build/delivery process.
- The upstream operator's public sync workflows merge `opendatahub-operator`
  `main → stable → rhoai`; inspection found no corresponding public DCH-repo
  sync workflow. Confirm the external DCH source/image sync and then register the
  RHOAI `rhoai-3.7-ea.1` module source and controller image. Do not infer that
  the operator-only sync builds or retags DCH images.
- Resolve the stale Kubebuilder `PROJECT` scope metadata through an approved
  Kubebuilder workflow; generated CRD scope is Cluster and the existing
  namespaced `DataConnectService` API was not moved or overwritten.

| PR area | Repository | Scope and current state |
| --- | --- | --- |
| Parent integration | `opendatahub-io/opendatahub-operator` | Local/uncommitted: API and v2/v3 preservation foundation, independent module handler/Platform enablement, parent RBAC, and Data readiness/release aggregation. Manifest source/image pins and broader lifecycle validation remain outstanding. |
| Independent DCH orchestration | `opendatahub-io/opendatahub-operator` | Local/uncommitted: DCH is registered as an independent sibling module with its own `Platform.spec.modules.dataconnecthub` entry; Feast remains independent. |
| DCH module implementation | `opendatahub-io/data-connect-hub` | Local/uncommitted: cluster-scoped `DataConnectHub` module API/controller and bootstrap packaging alongside the existing namespaced application API. |
| RHOAI delivery | `red-hat-data-services/rhods-operator` and applicable downstream module forks | Target: `rhoai-3.7-ea.1`; downstream DCH source/image synchronization is a follow-up because the public operator sync workflow covers only `opendatahub-operator`. |

The API foundation is not a deployable integration by itself. Keep the parent
work unmerged until its runtime dependencies and lifecycle tests are ready.

### Ownership evidence found on GitHub

- The current parent `internal/controller/modules/data/handler.go` uses
  `gvk.FeastOperator`, `default-feastoperator`, and Feast's module controller image.
- `manifests-config.yaml` maps `data` to `feast-module-operator/config/chart`.
- The Feast module currently manages Feature Store and Data Registry only; its
  API and controller contain no DCH orchestration.
- Merged parent [PR #4178](https://github.com/opendatahub-io/opendatahub-operator/pull/4178)
  renamed the module to Data while retaining Feast resources. In its
  [review discussion](https://github.com/opendatahub-io/opendatahub-operator/pull/4178#discussion_r4215131217),
  a maintainer identifies **RHOAIENG-95346** as the ticket for the planned Feast
  resource-identity change. This is a lead, not confirmation of a DCH ownership
  assignment; the ticket contents have not been verified.
- The approved [module onboarding ADR](https://github.com/opendatahub-io/architecture-decision-records/blob/main/architecture-decision-records/operator/ODH-ADR-Operator-0012-module-onboarding.md)
  and [detailed guide](https://github.com/opendatahub-io/architecture-decision-records/blob/main/architecture-decision-records/operator/design/module-onboarding-guide.md)
  specify a cluster-scoped module CR and allow the module to deploy operand
  controllers. They do not require a second cluster-scoped module CR for each
  operand controller.

## Goal

Onboard Data Connect Hub (DCH) as an independently managed capability within
the DSC Data grouping, alongside Feature Store and Data Registry. Follow the
parent platform's module contracts for installation, configuration, ownership,
readiness, upgrades, and removal.

The clarified direction is independent Feast and DCH modules orchestrated at
the ODH/RHOAI platform layer. `data` is their shared public DSC grouping, not a
requirement for Feast to own DCH. This supersedes the earlier proposal to expand
`feast-module-operator` into a shared Data orchestrator. New resource and field
names remain subject to the parent team's API review.

**Execution boundary:** this plan authorizes no live cluster changes. Development
and validation should use local rendering, unit tests, envtest, and an isolated
test cluster. Deployment to atris50 requires a separate request.

## Research baseline

The earlier read-only inspection of atris50 found:

- DSC `default-dsc`, using `datasciencecluster.opendatahub.io/v3`.
- RHOAI release `3.6.0`, with DSC status `Ready`.
- Served DSC versions `v2` and `v3`; storage version `v3`.
- Applications namespace `redhat-ods-applications`.
- `spec.components.data.featureStore.managementState: Managed`.
- `spec.components.data.dataRegistry: {}`, projected to `Removed`.
- No DCH field in the installed DSC Data schema.
- A DSC-owned `FeastOperator/default-feastoperator` with Feature Store enabled
  and Data Registry disabled.

These are historical observations, not a fresh assessment of the cluster.
Before implementation, pin and recheck the target ODH/RHOAI branches and module
revisions; the linked development patches have evolved since the original
runbook.

### Existing contracts and gaps

| Area | Observed implementation | Required work |
| --- | --- | --- |
| Public API | `DSCData` contains `featureStore` and `dataRegistry` | Add independent DCH management plus approved gateway and audience overrides |
| Data module | Parent handler currently renders Feast's chart and creates one FeastOperator CR | Register an independent sibling DCH handler in the platform operator |
| DCH application API | `DataConnectService` is namespaced | Provide DCH's own cluster-scoped module API while retaining the namespaced application API |
| DCH status | Conditions, observed generation, distribution, and releases already exist | Verify and connect them to parent readiness and upgrade contracts |
| DCH configuration | Standalone reads `opendatahub-dataconnecthub-config` | Only the platform-owned child reads `odh-dataconnecthub-config`; preserve standalone fallback and project approved DSC overrides |
| Packaging | DCH uses Kustomize | Supply parent-consumable manifests; both Kustomize and Helm are supported |

The actual DCH API marker and generated CRD declare `DataConnectService` as
namespaced. The root `AGENTS.md` description of it as a cluster-scoped singleton
needs correction as part of the implementation documentation update.

## Approved working public API

The names are approved for this implementation; module/source pins are still
pending:

```yaml
apiVersion: datasciencecluster.opendatahub.io/v3
kind: DataScienceCluster
metadata:
  name: default-dsc
spec:
  components:
    data:
      featureStore:
        managementState: Managed
      dataRegistry:
        managementState: Removed
      dataConnectHub:
        managementState: Managed
        # Optional overrides. Omit either field to use platform discovery.
        gateway:
          name: data-science-gateway
          namespace: openshift-ingress
        tokenReviewAudiences:
          - https://issuer.example.com
```

- Each capability has an independent `Managed` or `Removed` state.
- An omitted DCH capability resolves to `Removed`.
- DCH must work when both Feast capabilities are removed.
- Enabling or removing DCH must not change either Feast capability.
- `gateway` and `tokenReviewAudiences` are administrator overrides. When absent
  or empty, platform gateway and service-account issuer discovery apply.
- Keep discovered distribution, applications namespace, and default gateway
  projection in the internal control resource.
- Do not automatically expose the entire application CR spec through the DSC;
  define the supported platform-facing configuration deliberately.

## Proposed ownership architecture

Register DCH as a peer of the existing Feast-backed Data handler in the platform
module framework. Each handler reads its own configuration from the shared
`DSC.spec.components.data` object. The framework's `BuildModuleCR` returns one
CR per handler, so a separate handler gives DCH its own lifecycle and ownership.
`DataConnectHub` is the approved module kind, distinct from the application API.

```text
DataScienceCluster
  └── spec.components.data
        ├── featureStore / dataRegistry
        │     └── existing platform Data handler
        │           └── FeastOperator CR + feast-module-operator
        │                 └── Feast operator and workloads
        └── dataConnectHub
              └── new platform DCH handler
                    └── DataConnectHub module CR + DCH module controller
                          └── DataConnectService (namespaced)
                                └── REST / Flight and supporting resources
```

The platform's existing internal `Platform.spec.modules.data` continues to
enable the Feast-backed module only. DCH uses a separate internal module entry
`dataconnecthub`. Do not OR DCH into Feast enablement: a
DCH-only installation must not install or require Feast. Internal module names
do not have to mirror the public DSC grouping.

The DSC controller calls `PopulatePlatformModule` and `BuildModuleCR`. The
Platform controller calls `GetOperatorManifests` and uses the shared rendering,
image/config injection, deployment, and cleanup actions. Handlers describe the
resources; the framework applies them.

### Parent code map for implementation

Paths in this table are relative to `opendatahub-operator`:

| Location | Responsibility |
| --- | --- |
| `api/components/v1alpha1/data_types.go` | Public Data configuration, including approved gateway and TokenReview audience overrides. |
| `api/datasciencecluster/v2/datasciencecluster_conversion.go` | Preserve v3 DCH state and overrides through v2 reads/writes using `conversion.opendatahub.io/dataconnecthub-v3-state`. |
| `api/config/v1alpha2/platform_types.go` and Platform conversion code | Independent internal DCH enablement, enumeration, and version compatibility. |
| `internal/controller/modules/dataconnecthub/handler.go` (new) | Implement DCH's `PopulatePlatformModule`, `IsEnabled`, and `BuildModuleCR`; configure its own manifests, GVK, image injection, and status integration. |
| `internal/controller/modules/builtin/handlers.go` | Register the new handler and provisioning runlevel. |
| `internal/controller/datasciencecluster/datasciencecluster_controller_actions.go` | `provisionModuleCRsWith` builds each enabled handler's CR; DSC reconciliation applies the resulting resources. |
| `internal/controller/modules/modules_controller_actions.go` | Platform-side `provisionModules` gathers operator manifests; shared actions render/apply them. Also contains module status computation that must support Data aggregation. |
| `internal/controller/modules/base.go` | Shared module status, CR lookup/deletion, and manifest behavior; default CR lookups are cluster-scoped. |
| `internal/controller/modules/data/handler.go` | Existing Feast-backed integration; preserve its installation behavior and refactor its Data aggregate status writing as needed. |
| `manifests-config.yaml`, GVK/RBAC definitions, and applicable build configuration | Register DCH bootstrap manifests, controller/operand images, and permissions. |

The new handler should project only
`DSC.Spec.Components.Data.DataConnectHub.ManagementState` into its internal
enablement entry, normalizing empty to `Removed`. Feast's existing enablement
continues to depend only on Feature Store or Data Registry.

### Responsibilities

**ODH/RHOAI platform operator**

- Own the public DSC API and version conversion.
- Project Feast and DCH configuration into their separate module CRs.
- Install and configure both module operators through the module framework.
- Aggregate Data readiness and releases into DSC status.

**Existing Feast module**

- Continue managing Feature Store and Data Registry through the existing API.
- Remain enabled only while either Feast capability is managed.

**DCH operator**

- Reconcile its own cluster-scoped singleton module CR.
- Create/configure the platform-managed namespaced `DataConnectService` and
  aggregate its application status into the DCH module CR.
- Continue reconciling REST, Flight, networking, authentication, and supporting
  resources through the existing application controller.
- Distinguish platform-owned instances from independently created instances.

Use controller owner references and the parent's dynamic ownership conventions.
Each object must have one controller owner. Do not attach module lifecycle owner
references to shared CRDs or delete namespaces as routine capability cleanup.

No DCH orchestration changes are planned in `feast-module-operator` or the Feast
operand repository. MLflow's independent module contract and the platform's
existing handler framework are the primary references for DCH integration.

**Status aggregation needs an explicit parent change.** The current Data handler
writes `DataReady` and `status.components.data` using only Feast status. Preserve
independent per-module results, then compute the public Data aggregate once from
enabled Feast and DCH modules. Two handlers must not overwrite the same aggregate
field or condition. A removed Feast module must not block a ready DCH-only setup.

Expected lifecycle/status behavior:

| Feast capability enabled | DCH enabled | Expected behavior |
| --- | --- | --- |
| No | No | Both modules removed; Data removal reported using parent conventions. |
| Yes | No | Feast alone determines active Data readiness and releases. |
| No | Yes | DCH alone determines active Data readiness and releases; no Feast CR/operator is required. |
| Yes | Yes | Aggregate both enabled modules; each retains independent cleanup and reconciliation. |

If either enabled module is unhealthy, the public aggregate must reflect that
according to the parent's readiness/degradation rules. Aggregation must not
create an installation dependency: a Feast failure must not prevent DCH from
being reconciled. Keep stale-generation detection and platform-version checks
for each module and merge release information deterministically.

## Implementation phases

### 1. Establish the API and ownership contract

- [x] Select ODH `main` and RHOAI `rhoai-3.7-ea.1` as targets; existing Feast
      module sources remain pinned in the parent manifest configuration.
- [x] Confirm `dataConnectHub` as the DSC field.
- [x] Agree on `DataConnectHub`, `default-dataconnecthub`, and
      `components.platform.opendatahub.io/v1alpha1`.
- [x] Select independent platform-level DCH orchestration; DCH is a peer of
      Feast, and its module implementation belongs in `data-connect-hub`.
- [x] Use the approved `DataConnectHub` module API with a separate
      `components.platform.opendatahub.io` API group.
- [x] Use `default-dataconnectservice` in the parent-supplied applications
      namespace; namespace changes are serialized cleanup/recreation.
- [x] Refuse to adopt or overwrite an independently owned DCS with the reserved
      name; adoption is unsupported for the first release.
- [x] Document data retention and namespace-transition semantics: changing the
      applications namespace deletes/finalizes the old owned DCS before creating
      the new one; namespace-local secrets are not copied and must be provisioned
      in the destination namespace. External database contents are not deleted.

### 2. Extend the parent platform API and integration

- [x] Extend `api/components/v1alpha1/data_types.go` with the proposed DCH capability.
- [x] Keep public Data status on its existing aggregate type and add independent
      Feast/DCH readiness condition mappings.
- [x] Add `internal/controller/modules/dataconnecthub/handler.go` and register it
      in `internal/controller/modules/builtin/handlers.go`.
- [x] Project DCH into its own module CR and internal
      `Platform.spec.modules.dataconnecthub` enablement entry.
- [x] Preserve the independent DCH Platform enablement through the served
      Platform v1alpha1-to-v1alpha2 conversion.
- [x] Aggregate public Data status and releases across both independently
      enabled modules without having either handler overwrite the aggregate.
- [x] Register the module GVK, parent-operator RBAC, handler, and provisioning
      runlevel.
- [ ] Update manifest sources and related-image registrations for ODH and RHOAI.
- [x] Regenerate API deepcopy code, CRDs, RBAC, and API documentation.
- [x] Update ODH/RHOAI DSC samples and generated user-facing API documentation.

**Version compatibility:** DSC v2 currently represents the Feast capabilities
through `spec.components.feastoperator`. Define how the new v3 DCH state survives
a v3 → v2 → v3 round trip and an unrelated v2 update. Use the parent's accepted
preservation mechanism or an explicitly supported v2 field. Do not silently
drop DCH state during conversion.

### 3. Implement independent DCH orchestration

- [x] Implement independent child installation and capability projection.
- [x] Default omitted DCH state to `Removed` before projection.
- [x] Forward applications namespace, platform identity/version, default gateway,
      DSC gateway/audience overrides or discovered defaults, and child-related
      images.
- [x] Read child observed generation, readiness, degradation, and releases.
- [x] Report the independent module condition and aggregate `DataReady` consistently.
- [x] Keep the combined DCH manager available until the child control CR and
      managed operands have completed finalization, independently of Feast.

Use the platform framework's per-module installation, version handshake, and
two-phase removal for DCH. Verify DCH is not assigned a dependency on Feast in
the provisioning DAG.

### 4. Adapt DCH to the platform module contract

- [x] Scaffold the independent DCH module API with Kubebuilder and implement its
      controller in `dc-controller`, following repository generation rules.
- [x] Have the DCH module controller create/configure its platform-managed
      namespaced `DataConnectService` and observe application status.
- [x] Preserve the existing namespaced application API and standalone usage.
- [x] Align configuration with the parent `odh-dataconnecthub-config` version
      handshake, while retaining the legacy standalone ConfigMap fallback.
- [x] Project applications namespace, gateway reference, and service-account
      TokenReview audience explicitly.
- [x] Report `observedGeneration`, `Ready`, `ProvisioningSucceeded`, `Degraded`,
      and component releases with the semantics the parent consumes.
- [x] Report the `platform` release version only after application readiness.
- [x] Watch platform configuration changes and reconcile DCH and DCS resources.
- [x] Update the stale application/module scope guidance in repository `AGENTS.md`.

### 5. Package for ODH and RHOAI

- [x] Provide a parent-consumable Kustomize bundle: Deployment, ServiceAccount,
      RBAC, CRDs, and configuration, without a pre-created platform control CR.
- [x] Reuse Kustomize for ODH/RHOAI module manifests; retain the standalone Helm
      chart.
- [ ] Pin the module controller, REST, Flight, and proxy images in the parent for
      both ODH and RHOAI; the RHOAI controller image is not yet identified.
- [x] Verify standalone Kustomize and Helm renderings, cluster/namespace CRD scope,
      and absence of bundled platform control-CR instances.
- [x] Include the permissions and namespace configuration needed by the installed
      operator; rendered Kustomize and Helm bundles include their RBAC and config.
- [x] Ensure generated and bundled CRD schemas match.
- [x] Define the standalone Helm upgrade procedure: apply updated CRDs from
      `charts/crds/` before `helm upgrade`; Helm's `crds/` directory does not
      upgrade already-installed CRDs.

### 6. Preserve existing installations during migration

- [x] Add DCH as an independent module without transferring ownership of Feast
      resources or replacing its control CR.
- [x] Preserve `default-feastoperator`, existing Feature Store/Data Registry state,
      and associated user resources; DCH enablement/removal does not modify Feast.
- [x] Reject an existing independently owned DCH application instance with the
      reserved name; do not adopt or overwrite it.
- [x] Do not support adoption in the first release; report conflicts and preserve
      independently owned resources.
- [x] Assign one controller owner to the platform-managed DCS and reject an
      unowned same-name DCS to prevent competing controllers.
- [ ] Account for existing manifest paths, labels, release names, and cleanup logic.
- [ ] Define failure recovery, retries, and supported rollback boundaries.

## Validation plan

### Local and API tests

- [x] `data-connect-hub/dc-controller`: `make test` and `make lint` pass.
- [x] Rendered the DCH Kustomize bootstrap and Helm chart locally; both include
      the cluster-scoped module CRD and namespaced application CRDs, and neither
      includes a pre-created `DataConnectHub` control-CR instance.
- [x] `opendatahub-operator`: generated API/deepcopy/CRD/docs, `make fmt lint`,
      `make build`, `make manifests ODH_PLATFORM_TYPE=rhoai`, and `make bundle-all`
      pass. A `make unit-test-operator` run restricted to changed API/handler
      packages also passes. Targeted DCH handler, Feast handler, module registry,
      Platform v1/v2 conversion, DSC v2/v3 conversion, and Data aggregate tests pass.
- [ ] Render ODH and RHOAI bundles and validate CRD schemas, namespace placement,
      RBAC, image substitutions, and absence of bundled control-CR instances.
- [x] Test all eight combinations of Feature Store, Data Registry, and DCH enabled
      or disabled, plus omitted-field defaulting.
- [x] Verify that enabling DCH leaves Feast enablement and projected capabilities
      unchanged in the handler unit tests.
- [x] Test v2/v3 conversion, including an unrelated v2 write preserving DCH state
      and gateway/audience overrides; also test Platform v1alpha1/v1alpha2
      preservation of the internal DCH key.
- [x] Test child status aggregation for pending/stale, ready, degraded, and removal;
      the application controller's failed-provisioning path is covered separately.
- [x] Test platform version alignment and platform ConfigMap change reconciliation.
- [x] Test module-before-operator finalizer ordering and idempotent cleanup; the
      external database and user-owned secrets are not deleted or migrated.
- [x] Run relevant Go lint, generation, and controller test targets for each
      changed repository. Rust sources were unchanged, so Rust checks were not run.

The full parent module-manifest compliance suite is not currently runnable from
this checkout: `opendatahub-operator/opt/manifests` does not contain fetched
module sources, and the DCH source entry cannot be pinned until the DCH
implementation revision and RHOAI image are available. Do not count targeted
unit tests or local DCH chart rendering as a substitute for combined parent
bundle validation.

### Isolated integration tests

- [ ] Fresh installation with only DCH enabled.
- [ ] Fresh installation with all Data capabilities enabled.
- [ ] Enable and disable each capability while siblings remain active.
- [ ] Remove all capabilities and verify ordered cleanup without stuck finalizers.
- [ ] Upgrade an existing Feast-only DSC and enable DCH without recreating Feast.
- [ ] Upgrade with independently created DCH instances present.
- [ ] Validate custom applications namespaces, ODH/RHOAI configuration, and
      related-image overrides.
- [ ] Verify platform-managed REST and Flight readiness and authenticated access.

## Completion criteria

The integration is complete when an administrator can manage DCH through
`spec.components.data.<approved-field>` on both ODH and RHOAI, independently of
Feast capabilities, with accurate DSC readiness and release reporting. Fresh
installation, upgrade/coexistence, configuration updates, version
conversion, and removal must pass the validation above without manual cluster
corrections or unintended deletion of user resources.

## References

- [RHOAI Data API](https://github.com/red-hat-data-services/rhods-operator/blob/main/api/components/v1alpha1/data_types.go)
- [RHOAI Data handler](https://github.com/red-hat-data-services/rhods-operator/blob/main/internal/controller/modules/data/handler.go)
- [ODH Data API](https://github.com/opendatahub-io/opendatahub-operator/blob/main/api/components/v1alpha1/data_types.go)
- [Parent module framework](https://github.com/red-hat-data-services/rhods-operator/blob/main/internal/controller/modules/README.md)
- [Parent manifest registration](https://github.com/red-hat-data-services/rhods-operator/blob/main/manifests-config.yaml)
- [Feast module operator](https://github.com/red-hat-data-services/feast-module-operator)
- [AI Gateway operator](https://github.com/red-hat-data-services/ai-gateway-operator)
- [MLflow operator](https://github.com/red-hat-data-services/mlflow-operator)
- [Original DSC v3 branch](https://github.com/lburgazzoli/opendatahub-operator/tree/RHOAIENG-94812-DSC-v3)
- [Parent capability projection PR](https://github.com/lburgazzoli/opendatahub-operator/pull/2)
- [Feast module capabilities PR](https://github.com/opendatahub-io/feast-module-operator/pull/27)
- [Feast platform capabilities and RBAC PR](https://github.com/opendatahub-io/feast/pull/218)
- [DCH application API](../../dc-controller/api/dataconnecthub/v1alpha1/dataconnectservice_types.go)
- [DCH controller](../../dc-controller/internal/controller/dataconnectservice_controller.go)

The original PRs explain the design's evolution. Use pinned current source and
generated schemas as the implementation contract; older namespace options and
deployment workarounds in the initial runbook are not the target architecture.
