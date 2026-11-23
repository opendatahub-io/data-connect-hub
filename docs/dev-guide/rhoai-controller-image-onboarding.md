# RHOAI dc-controller image handoff

This downstream build/release preparation can run in parallel with the upstream
shared Data-module integration; it does not need the new validation cluster.
Controller PR and push PipelineRun drafts have been prepared and structurally
validated locally. No downstream component, registry repository, or release was
created. Registry-access checks still require build-team credentials.

## Request to build/release engineering

Please onboard the DCH Go controller alongside the existing REST and Flight
components. Confirm the release target and approved architectures before
publishing the pipeline or provisioning resources.

| Input | Proposed value |
| --- | --- |
| Tracking | RHOAIENG-79055; controller integration tracked with RHOAIENG-95346 |
| Source | `https://github.com/red-hat-data-services/data-connect-hub` |
| Source/release branch | `rhoai-3.7-ea.1` (working assumption, needs confirmation) |
| Konflux namespace | `rhoai-tenant` |
| Application | `rhoai-v3-7-ea-1` |
| Component | `odh-data-connect-hub-controller-v3-7-ea-1` |
| Build service account | `build-pipeline-odh-data-connect-hub-controller-v3-7-ea-1` |
| Containerfile | `dc-controller/Containerfile.konflux` |
| Build context | Repository root (`.`) |
| Dependency prefetch | `[{"type":"gomod","path":"dc-controller"}]` |
| Proposed output repository | `quay.io/rhoai/odh-data-connect-hub-controller-rhel9` |
| Platform image variable | `RELATED_IMAGE_ODH_DATA_CONNECT_HUB_CONTROLLER_IMAGE` |

The proposed PipelineRun follows the release branch's existing component pattern
and targets the shared `pipelines/multi-arch-container-build.yaml`. It requests
the same four architecture families as DCH REST/Flight. Architecture support
for the Go controller still needs build/release approval and successful builds.

Build/release engineering must arrange the actual Component/Application
configuration, service account and Git/registry credentials, repository access,
integration tests, signing/promotion, enterprise-contract policy, and approved
release configuration. The local YAML is not a substitute for provisioning.

## Publication sequence

1. Confirm the release target and provision the controller Component against the
   existing downstream controller source/Containerfile. An initial build can
   validate packaging while the upstream PRs are reviewed. Synchronize the tested
   DCH controller changes before the final feature/release build; upstream merge
   and cluster validation do not block preparing the build configuration.
2. Add controller PR/push PipelineRuns in
   `red-hat-data-services/konflux-central`, under
   `pipelineruns/data-connect-hub/.tekton/`. Release branches are independent of
   `main`; follow the repository's current PR pipeline identity/template rules.
   Let automation synchronize `.tekton/` into the DCH component repository.
3. Verify a hermetic controller build on every approved architecture. The
   existing Containerfile has not been validated by a downstream Konflux build
   in this work. Confirm dependency-prefetch consumption and the Go/FIPS build
   configuration; do not copy the Rust services' FIPS exception.
4. Verify required scans, SBOM/provenance, signing, and promotion. Record the
   resulting approved controller image digest.
5. Use RHOAI release automation to add the controller to the target
   `RHOAI-Build-Config` payload. The existing REST-image onboarding PR
   [#30670](https://github.com/red-hat-data-services/RHOAI-Build-Config/pull/30670)
   is a precedent for the image env entry, repository mapping, and bundle
   provenance changes. Configure image updates/nudging for subsequent builds.
6. Confirm the final related-images inventory and supported Helm payloads include
   the controller, REST, Flight, and proxy images. REST/Flight were removed from
   3.6; their presence in another branch does not prove target-release delivery.
7. Synchronize/pin the matching Feast module and platform integration. Confirm
   the controller image env reaches Feast and replaces dc-controller's Deployment
   image; confirm REST/Flight/proxy image env variables reach dc-controller.
8. Remove the temporary upstream RHOAI controller-image validation exception
   only after the downstream payload actually publishes the controller image.
9. Validate a fresh install, upgrade, removal/re-enable, and disconnected
   mirroring using the built release payload.

## Boundaries

- `odh-mod-arch-data-connect-hub` is the dashboard image, not this Go controller.
- No separate DCH OLM subscription/bundle or Platform module is introduced:
  the existing Feast module installs dc-controller for the DCH capability.
- PostgreSQL and `dch-database-config` remain administrator-provided
  prerequisites; this image onboarding does not provision them.
- The controller's Quay repository was not verified locally because the relevant
  registry credentials were unavailable. Pipeline validation skipped that check.

See the [upstream onboarding plan](dsc-data-onboarding-plan.md) for implementation
status and cross-repository dependencies.
