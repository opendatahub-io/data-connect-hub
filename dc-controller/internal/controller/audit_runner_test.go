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
	"sync/atomic"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
)

// mockAuditClient is a test double for AuditClient.
type mockAuditClient struct {
	calls   atomic.Int32
	auditFn func(ctx context.Context) error
}

func (m *mockAuditClient) AuditDataConnectionTypes(ctx context.Context) error {
	m.calls.Add(1)
	if m.auditFn != nil {
		return m.auditFn(ctx)
	}
	return nil
}

func TestAuditRunnerNeedsLeaderElection(t *testing.T) {
	assert.True(t, (&AuditRunner{}).NeedLeaderElection())
}

func TestAuditRunnerTicksIndependentlyOfReconcile(t *testing.T) {
	mock := &mockAuditClient{}
	runner := &AuditRunner{Client: mock, Interval: 10 * time.Millisecond}

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- runner.Start(ctx) }()

	assert.Eventually(t, func() bool { return mock.calls.Load() >= 3 }, time.Second, 5*time.Millisecond,
		"expected the runner to call the audit client multiple times without any Reconcile call")

	cancel()
	assert.NoError(t, <-done)
}

func TestAuditRunnerStopsOnContextCancel(t *testing.T) {
	mock := &mockAuditClient{}
	runner := &AuditRunner{Client: mock, Interval: time.Hour}

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- runner.Start(ctx) }()

	cancel()

	select {
	case err := <-done:
		assert.NoError(t, err)
	case <-time.After(time.Second):
		t.Fatal("runner did not stop after context cancellation")
	}
}

func TestAuditRunnerLogsAndContinuesOnError(t *testing.T) {
	mock := &mockAuditClient{auditFn: func(context.Context) error { return assert.AnError }}
	runner := &AuditRunner{Client: mock, Interval: 10 * time.Millisecond}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() { _ = runner.Start(ctx) }()

	assert.Eventually(t, func() bool { return mock.calls.Load() >= 3 }, time.Second, 5*time.Millisecond,
		"expected the runner to keep ticking after an audit error")
}
