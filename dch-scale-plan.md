## Autoscaling Implementation Plan

### 1. Define the CR contract

Add optional autoscaling under:

- [x] `spec.restService.autoScaling`
- [x] `spec.flightService.autoScaling`

Fields:

- [x] `minReplicas`: required
- [x] `maxReplicas`: required
- [x] `pollingInterval`: optional, default `15`
- [x] `cooldownPeriod`: optional, default `300`
- [x] `triggers[]`:
  - [x] `name`
  - [x] `serverAddress`: required
  - [x] `query`
  - [x] `target.type`
  - [x] `target.value`

Validation:

- [x] Reject `replicas` combined with `autoScaling`.
- [x] Reject triggers without `serverAddress`.
- [x] Validate replica bounds and trigger fields.
- [x] Keep autoscaling disabled when omitted.

### 2. Update generated API artifacts

- [x] Modify `dc-controller/api/dataconnecthub/v1alpha1/dataconnectservice_types.go`.

Regenerate:

- [x] CRD manifests
- [x] Deepcopy code
- [x] RBAC manifests if needed

### 3. Document Prometheus configuration

- [x] Add user documentation explaining how to obtain the OpenShift Thanos
`serverAddress` and configure it in each trigger.

The controller will not:

- [x] Discover OpenShift Routes.
- [x] Read a platform ConfigMap for the address.
- [x] Apply a hard-coded default.

### 4. Render autoscaling resources

When configured, create owned resources:

- [x] ServiceAccount
- [x] Service-account token Secret
- [x] `TriggerAuthentication`
- [x] Required metrics-reader Role and RoleBinding
- [x] REST `ScaledObject`
- [x] Flight `ScaledObject`

- [x] Use per-`DataConnectService` names to avoid collisions. Target the actual
  rendered Deployment names.

### 5. Integrate reconciliation

- [x] Reconcile autoscaling resources after service Deployments are rendered.
- [x] Use idempotent create/update behavior.
- [x] Treat OpenShift CMA/KEDA as a prerequisite.
- [x] Do not install or manage CMA/KEDA.
- [x] Detect missing KEDA APIs and report a clear degraded status.
- [ ] Observe `ScaledObject` readiness where practical.

### 6. Prevent replica conflicts

- [x] Reject configurations where either service specifies both:

```yaml
replicas: ...
autoScaling: ...
```

- [x] This prevents the controller and KEDA from competing over Deployment replica
  counts.

### 7. Clean up removed autoscaling configuration

When an `autoScaling` block is removed, explicitly delete its:

- [x] `ScaledObject`
- [x] `TriggerAuthentication`
- [x] Secret
- [x] ServiceAccount
- [x] Role
- [x] RoleBinding

- [x] Owner references remain as cleanup protection when the entire CR is deleted.

### 8. Update permissions and watches

- [x] Add controller permissions for:

- [x] KEDA `ScaledObject`s
- [x] KEDA `TriggerAuthentication`s
- [x] ServiceAccounts
- [x] Secrets
- [x] Roles
- [x] RoleBindings

- [ ] Watch owned autoscaling resources if required for reconciliation.

### 9. Resolve OOB installation

- [x] Keep autoscaling disabled in the default OOB CR.
- [ ] Identify the authoritative OOB CR installation path.
- [x] Document CMA as a prerequisite.
- [x] Do not package CMA/KEDA installation with DCH.

### 10. Test

Cover:

- [x] CR validation and defaults
- [x] Missing `serverAddress`
- [x] Replica/autoscaling conflicts
- [x] REST and Flight `ScaledObject` rendering
- [ ] Resource cleanup
- [ ] Missing KEDA APIs
- [ ] Idempotency and ownership
- [ ] CMA-backed REST and Flight e2e scaling

### 11. Verify

- [x] Run controller generation, formatting, vet, lint, unit tests, and generated
  manifest validation.
- [ ] Run CMA-backed e2e tests.
- [ ] Inspect the OOB bundle.


### 12. Documentation
- [x] The documentation file: docs/user-guide/autoscaling-ocp.md since instructions will be OpenShift specific.
- [x] Follow style in: https://github.com/opendatahub-io/data-connect-hub/blob/main/docs/user-guide/tracing.md
- [x] Bash commands are in `bash` blocks and can be copied, pasted, and run by users.
- [x] Add commands for users to verify all the resources created for autoScaling for the example in the document.

### 13. Update Complete Task In Plan
- [x] Go through each item in the plan and update its status. Completed items are marked with `[x]`; incomplete items are marked with `[ ]`.
