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

package support

import (
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/utils/ptr"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// TrustedMonitoringEndpoint is the only serverAddress the controller accepts
// for autoscaling triggers. Port 9092 is the cluster monitoring tenancy query
// API, the only endpoint the autoscaling service-account token may be sent to.
const TrustedMonitoringEndpoint = "https://thanos-querier.openshift-monitoring.svc.cluster.local:9092"

// EnableRESTAutoScaling patches a REST autoscaling trigger with the given
// serverAddress and schedules restoration of the original replica management.
func (e *E2EFixture) EnableRESTAutoScaling(serverAddress string) *dchv1alpha1.DataConnectService {
	DeferCleanup(e.DisableRESTAutoScaling)
	By("patching REST autoscaling configuration")
	return e.patchDCS(func(dcs *dchv1alpha1.DataConnectService) {
		dcs.Spec.RestService.Replicas = nil
		dcs.Spec.RestService.AutoScaling = &dchv1alpha1.AutoScalingConfig{
			MinReplicas: ptr.To(int32(1)),
			MaxReplicas: ptr.To(int32(2)),
			Triggers: []dchv1alpha1.AutoscalingTrigger{{
				Name:          "rest_request_rate",
				ServerAddress: serverAddress,
				Query:         "sum(rate(dch_rest_requests_total[2m]))",
				Target:        dchv1alpha1.AutoscalingTarget{Type: "Value", Value: "0.4"},
			}},
		}
	})
}

// DisableRESTAutoScaling removes the REST autoscaling configuration, restores
// the fixed replica count, and waits for the service to become Ready again.
func (e *E2EFixture) DisableRESTAutoScaling() {
	current := &dchv1alpha1.DataConnectService{}
	Expect(e.k8sClient.Get(e.ctx, e.dcsKey, current)).To(Succeed())
	if current.Spec.RestService != nil && current.Spec.RestService.AutoScaling != nil {
		By("removing REST autoscaling configuration")
		patched := e.patchDCS(func(dcs *dchv1alpha1.DataConnectService) {
			dcs.Spec.RestService.AutoScaling = nil
			dcs.Spec.RestService.Replicas = ptr.To(int32(1))
		})
		e.waitForDCSReady(patched.Generation, 1, 1)
	}
}

// AssertRESTAutoScalingRejected verifies the DataConnectService reports the
// serverAddress validation failure and that no autoscaling resources exist.
func (e *E2EFixture) AssertRESTAutoScalingRejected(patched *dchv1alpha1.DataConnectService) {
	By("waiting for the DataConnectService to report the invalid serverAddress")
	Eventually(func(g Gomega) {
		current := &dchv1alpha1.DataConnectService{}
		g.Expect(e.k8sClient.Get(e.ctx, e.dcsKey, current)).To(Succeed())
		g.Expect(current.Status.ObservedGeneration).To(Equal(patched.Generation))
		g.Expect(current.Status.Phase).To(Equal("Error"))

		degraded := meta.FindStatusCondition(current.Status.Conditions, "Degraded")
		g.Expect(degraded).NotTo(BeNil())
		g.Expect(degraded.Status).To(Equal(metav1.ConditionTrue))
		g.Expect(degraded.Reason).To(Equal("InvalidAutoscaling"))
		g.Expect(degraded.Message).To(ContainSubstring("serverAddress"))
	}, 2*time.Minute, time.Second).Should(Succeed())

	e.AssertNoAutoscalingResources()
}

// AssertRESTAutoScalingValidationCleared verifies a corrected serverAddress
// clears the validation failure and reconciliation proceeds to the autoscaling
// provider check, which is expected to be unavailable in the Kind cluster.
func (e *E2EFixture) AssertRESTAutoScalingValidationCleared(patched *dchv1alpha1.DataConnectService) {
	By("waiting for the corrected serverAddress to clear the validation failure")
	Eventually(func(g Gomega) {
		current := &dchv1alpha1.DataConnectService{}
		g.Expect(e.k8sClient.Get(e.ctx, e.dcsKey, current)).To(Succeed())
		g.Expect(current.Status.ObservedGeneration).To(Equal(patched.Generation))

		degraded := meta.FindStatusCondition(current.Status.Conditions, "Degraded")
		g.Expect(degraded).NotTo(BeNil())
		g.Expect(degraded.Reason).To(Equal("AutoscalingProviderUnavailable"))
		g.Expect(degraded.Message).NotTo(ContainSubstring("serverAddress"))
	}, 2*time.Minute, time.Second).Should(Succeed())

	e.AssertNoAutoscalingResources()
}

// AssertNoAutoscalingResources verifies no autoscaling objects were created
// for the DataConnectService.
func (e *E2EFixture) AssertNoAutoscalingResources() {
	By("verifying that no autoscaling resources were created")
	saErr := e.k8sClient.Get(e.ctx, types.NamespacedName{
		Name: dcsName + "-autoscaling-sa", Namespace: applicationNamespace,
	}, &corev1.ServiceAccount{})
	Expect(apierrors.IsNotFound(saErr)).To(BeTrue(), "autoscaling ServiceAccount must not exist")
	secretErr := e.k8sClient.Get(e.ctx, types.NamespacedName{
		Name: dcsName + "-autoscaling-token", Namespace: applicationNamespace,
	}, &corev1.Secret{})
	Expect(apierrors.IsNotFound(secretErr)).To(BeTrue(), "autoscaling token Secret must not exist")
}
