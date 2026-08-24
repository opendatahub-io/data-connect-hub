/*
Copyright 2026.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package controller

import (
	"context"
	"time"

	logf "sigs.k8s.io/controller-runtime/pkg/log"
)

// DefaultAuditInterval is how often AuditRunner calls the audit endpoint
// when Interval is left unset.
const DefaultAuditInterval = 10 * time.Minute

// AuditRunner periodically calls the REST service's internal audit endpoint,
// independent of any CR reconciliation. It is registered with the manager
// as a Runnable so DCT flight-capability flags keep resyncing with
// flight-service even during long stretches with no reconcile-triggering
// event (CR change, owned-Deployment change, etc).
type AuditRunner struct {
	Client   AuditClient
	Interval time.Duration
}

// NeedLeaderElection ensures only the elected leader runs the periodic
// audit, avoiding duplicate calls from standby replicas.
func (a *AuditRunner) NeedLeaderElection() bool {
	return true
}

// Start implements manager.Runnable. It blocks until ctx is cancelled.
func (a *AuditRunner) Start(ctx context.Context) error {
	interval := a.Interval
	if interval <= 0 {
		interval = DefaultAuditInterval
	}
	log := logf.FromContext(ctx).WithName("audit-runner")

	ticker := time.NewTicker(interval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
			if err := a.Client.AuditDataConnectionTypes(ctx); err != nil {
				log.Error(err, "periodic data-connection-types audit failed")
			}
		}
	}
}
