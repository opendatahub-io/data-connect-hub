# E2E Test Conventions

- Keep `e2e_test.go` as a concise suite outline: readable `Describe`/`Context`/`It` names and short calls to `support.E2EFixture` methods. Do not put Kubernetes operations, config values, polling loops, or detailed assertions there.
- Keep each `It` focused on one behavior. Put setup, implementation, assertions, and reusable scenario helpers in `support/`.
- Keep generic test infrastructure in `test/utils`; keep DCH E2E-specific helpers in `test/e2e/support`.
- Specs that mutate shared DCS or deployment state must use `DeferCleanup` to restore it and wait for reconciliation/readiness, so later specs do not inherit state.
- Keep this suite limited to controller/service reconciliation and its PostgreSQL system dependency; do not add a tenant datasource.
