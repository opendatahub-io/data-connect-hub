//go:build e2e
// +build e2e

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

package e2e

import (
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/opendatahub-io/data-connect-hub/dc-controller/test/e2e/support"
)

var _ = Describe("dc-controller", Ordered, func() {
	SetDefaultEventuallyTimeout(2 * time.Minute)
	SetDefaultEventuallyPollingInterval(time.Second)

	var fixture *support.E2EFixture

	BeforeAll(func() {
		fixture = support.NewE2EFixture()
		fixture.Setup(managerImage)
	})

	AfterAll(func() {
		if fixture != nil {
			fixture.Cleanup()
		}
	})

	AfterEach(func() {
		if fixture != nil {
			fixture.DumpDiagnostics()
		}
	})

	Context("controller startup", func() {
		It("starts the controller manager", func() {
			fixture.AssertControllerReady()
		})
	})

	Context("DataConnectService reconciliation", func() {
		It("creates a DataConnectService and waits for Ready", func() {
			fixture.CreateDataConnectService()
		})

		It("reconciles REST replica changes", func() {
			fixture.ReconcileRESTReplicas(2)
		})

		It("reconciles Flight replica changes", func() {
			fixture.ReconcileFlightReplicas(2)
		})

		Context("Flight connector configuration", func() {
			It("disables an additional connector", func() {
				fixture.DisableFlightConnector("postgres")
			})

			It("overrides connector defaults", func() {
				fixture.OverrideFlightConnectorDefaults("postgres")
			})
		})
	})

	Context("metrics endpoint", func() {
		It("serves metrics", func() {
			fixture.AssertMetricsEndpoint()
			// +kubebuilder:scaffold:e2e-metrics-webhooks-readiness
		})
	})

	// +kubebuilder:scaffold:e2e-webhooks-checks
})
