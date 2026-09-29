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
	"github.com/pelletier/go-toml/v2"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// CreateDataConnectService creates the service and waits for its initial Ready status.
func (e *E2EFixture) CreateDataConnectService() {
	initialRestReplicas := int32(1)
	By("creating the DataConnectService")
	dcs := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: dcsName, Namespace: applicationNamespace},
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService:   &dchv1alpha1.ServiceOverrides{Replicas: &initialRestReplicas},
			FlightService: &dchv1alpha1.FlightServiceConfig{},
		},
	}
	Expect(e.k8sClient.Create(e.ctx, dcs)).To(Succeed())
	Expect(dcs.UID).NotTo(BeEmpty())

	By("providing the REST service CA to the REST pod")
	flightCAKey := types.NamespacedName{Name: "dch-flight-service-ca", Namespace: applicationNamespace}
	var flightCA *corev1.ConfigMap
	Eventually(func(g Gomega) {
		flightCA = &corev1.ConfigMap{}
		g.Expect(e.k8sClient.Get(e.ctx, flightCAKey, flightCA)).To(Succeed())
	}, 2*time.Minute, time.Second).Should(Succeed())
	flightCABase := flightCA.DeepCopy()
	if flightCA.Data == nil {
		flightCA.Data = make(map[string]string)
	}
	flightCA.Data["service-ca.crt"] = string(e.serviceCA.certificatePEM)
	Expect(e.k8sClient.Patch(e.ctx, flightCA, client.MergeFrom(flightCABase))).To(Succeed())

	By("waiting for the initial reconcile to become Ready")
	e.waitForDCSReady(dcs.Generation, initialRestReplicas, 1)
}

// ReconcileRESTReplicas patches the REST replica count and verifies reconciliation.
func (e *E2EFixture) ReconcileRESTReplicas(replicas int32) {
	By("patching REST replicas")
	patched := e.patchDCS(func(dcs *dchv1alpha1.DataConnectService) {
		dcs.Spec.RestService.Replicas = &replicas
	})

	By("waiting for the REST replica change to reconcile and become Ready")
	e.waitForDCSReady(patched.Generation, replicas, 1)
}

// ReconcileFlightReplicas patches the Flight replica count and verifies reconciliation.
func (e *E2EFixture) ReconcileFlightReplicas(replicas int32) {
	By("patching Flight replicas")
	patched := e.patchDCS(func(dcs *dchv1alpha1.DataConnectService) {
		dcs.Spec.FlightService.Replicas = &replicas
	})

	By("waiting for the Flight replica change to reconcile and become Ready")
	e.waitForDCSReady(patched.Generation, 2, replicas)
}

// DisableFlightConnector disables the named connector and verifies its rendered configuration.
func (e *E2EFixture) DisableFlightConnector(name string) {
	disabled := false
	e.ReconcileFlightConnectorConfig(
		[]dchv1alpha1.ConnectorConfig{{Name: name, Enabled: &disabled}},
		map[string]map[string]any{
			name:      {"enabled": false},
			"default": defaultFlightConnectorSettings(),
		},
	)
}

// OverrideFlightConnectorDefaults overrides the named connector's timeout defaults and verifies the rendered config.
func (e *E2EFixture) OverrideFlightConnectorDefaults(name string) {
	enabled := true
	e.ReconcileFlightConnectorConfig(
		[]dchv1alpha1.ConnectorConfig{{
			Name:              name,
			Enabled:           &enabled,
			ConnectionTimeout: &metav1.Duration{Duration: 45 * time.Second},
			RequestTimeout:    &metav1.Duration{Duration: 90 * time.Second},
			ReadTimeout:       &metav1.Duration{Duration: 75 * time.Second},
		}},
		map[string]map[string]any{
			name: {
				"enabled":                 true,
				"connection_timeout_secs": int64(45),
				"request_timeout_secs":    int64(90),
				"read_timeout_secs":       int64(75),
			},
			"default": defaultFlightConnectorSettings(),
		},
	)
}

func defaultFlightConnectorSettings() map[string]any {
	return map[string]any{
		"enabled":                 true,
		"connection_timeout_secs": int64(10),
		"request_timeout_secs":    int64(30),
		"read_timeout_secs":       int64(30),
	}
}

// ReconcileFlightConnectorConfig applies connector overrides and checks the rendered Flight config.
func (e *E2EFixture) ReconcileFlightConnectorConfig(
	connectors []dchv1alpha1.ConnectorConfig,
	expected map[string]map[string]any,
) {
	DeferCleanup(e.ResetFlightConnectorConfig)
	By("patching Flight connector configuration")
	patched := e.patchDCS(func(dcs *dchv1alpha1.DataConnectService) {
		dcs.Spec.FlightService.Connectors = connectors
	})

	e.waitForDCSObservedGeneration(patched.Generation)
	e.useRecreateFlightDeploymentStrategy()

	By("waiting for connector configuration to reconcile and become Ready")
	e.waitForDCSReady(patched.Generation, 2, 2)
	e.assertFlightConnectorConfig(expected)
}

// ResetFlightConnectorConfig restores the default connector configuration and Flight deployment strategy.
func (e *E2EFixture) ResetFlightConnectorConfig() {
	current := &dchv1alpha1.DataConnectService{}
	Expect(e.k8sClient.Get(e.ctx, e.dcsKey, current)).To(Succeed())
	if current.Spec.FlightService != nil && len(current.Spec.FlightService.Connectors) > 0 {
		By("restoring default Flight connector configuration")
		patched := e.patchDCS(func(dcs *dchv1alpha1.DataConnectService) {
			dcs.Spec.FlightService.Connectors = nil
		})
		e.waitForDCSObservedGeneration(patched.Generation)
		e.useRecreateFlightDeploymentStrategy()
		e.waitForDCSReady(patched.Generation, 2, 2)
	}
	e.restoreFlightDeploymentStrategy()
}

func (e *E2EFixture) waitForDCSObservedGeneration(generation int64) {
	Eventually(func(g Gomega) {
		observed := &dchv1alpha1.DataConnectService{}
		if err := e.k8sClient.Get(e.ctx, e.dcsKey, observed); err != nil {
			g.Expect(err).NotTo(HaveOccurred())
			return
		}
		g.Expect(observed.Status.ObservedGeneration).To(Equal(generation))
	}, 5*time.Minute, 2*time.Second).Should(Succeed())
}

func (e *E2EFixture) useRecreateFlightDeploymentStrategy() {
	By("using a Recreate rollout for the single-node Kind Flight deployment")
	flightDeployment := &appsv1.Deployment{}
	flightDeploymentKey := types.NamespacedName{Name: e.flightServiceName, Namespace: applicationNamespace}
	Expect(e.k8sClient.Get(e.ctx, flightDeploymentKey, flightDeployment)).To(Succeed())
	if e.originalFlightDeploymentStrategy == nil {
		e.originalFlightDeploymentStrategy = flightDeployment.Spec.Strategy.DeepCopy()
	}
	if flightDeployment.Spec.Strategy.Type == appsv1.RecreateDeploymentStrategyType {
		return
	}
	flightDeploymentBase := flightDeployment.DeepCopy()
	flightDeployment.Spec.Strategy = appsv1.DeploymentStrategy{Type: appsv1.RecreateDeploymentStrategyType}
	Expect(e.k8sClient.Patch(e.ctx, flightDeployment, client.MergeFrom(flightDeploymentBase))).To(Succeed())
}

func (e *E2EFixture) restoreFlightDeploymentStrategy() {
	if e.originalFlightDeploymentStrategy == nil {
		return
	}

	flightDeployment := &appsv1.Deployment{}
	flightDeploymentKey := types.NamespacedName{Name: e.flightServiceName, Namespace: applicationNamespace}
	Expect(e.k8sClient.Get(e.ctx, flightDeploymentKey, flightDeployment)).To(Succeed())
	flightDeploymentBase := flightDeployment.DeepCopy()
	flightDeployment.Spec.Strategy = *e.originalFlightDeploymentStrategy.DeepCopy()
	Expect(e.k8sClient.Patch(e.ctx, flightDeployment, client.MergeFrom(flightDeploymentBase))).To(Succeed())
	e.originalFlightDeploymentStrategy = nil
}

func (e *E2EFixture) assertFlightConnectorConfig(expected map[string]map[string]any) {
	By("verifying the rendered Flight connector configuration")
	flightConfig := &corev1.ConfigMap{}
	flightConfigKey := types.NamespacedName{Name: e.flightServiceName + "-config", Namespace: applicationNamespace}
	Expect(e.k8sClient.Get(e.ctx, flightConfigKey, flightConfig)).To(Succeed())
	var parsedConfig struct {
		Connectors map[string]map[string]any `toml:"connectors"`
	}
	Expect(toml.Unmarshal([]byte(flightConfig.Data["config.toml"]), &parsedConfig)).To(Succeed())
	for name, expectedValues := range expected {
		connector, found := parsedConfig.Connectors[name]
		Expect(found).To(BeTrue(), "expected [connectors.%s] in Flight config.toml", name)
		for key, expectedValue := range expectedValues {
			Expect(connector).To(HaveKey(key), "expected %q in [connectors.%s]", key, name)
			Expect(connector[key]).To(Equal(expectedValue), "unexpected %q in [connectors.%s]", key, name)
		}
	}
}

func (e *E2EFixture) patchDCS(update func(*dchv1alpha1.DataConnectService)) *dchv1alpha1.DataConnectService {
	current := &dchv1alpha1.DataConnectService{}
	Expect(e.k8sClient.Get(e.ctx, e.dcsKey, current)).To(Succeed())
	base := current.DeepCopy()
	update(current)
	Expect(e.k8sClient.Patch(e.ctx, current, client.MergeFrom(base))).To(Succeed())

	patched := &dchv1alpha1.DataConnectService{}
	Expect(e.k8sClient.Get(e.ctx, e.dcsKey, patched)).To(Succeed())
	Expect(patched.Generation).To(BeNumerically(">", base.Generation))
	return patched
}

func (e *E2EFixture) waitForDCSReady(expectedGeneration int64, expectedRestReplicas, expectedFlightReplicas int32) {
	Eventually(func(g Gomega) {
		current := &dchv1alpha1.DataConnectService{}
		if err := e.k8sClient.Get(e.ctx, e.dcsKey, current); err != nil {
			g.Expect(err).NotTo(HaveOccurred())
			return
		}
		g.Expect(current.Status.ObservedGeneration).To(Equal(expectedGeneration))
		g.Expect(current.Status.Phase).To(Equal("Ready"))

		readyCondition := false
		provisioningSucceeded := false
		for _, condition := range current.Status.Conditions {
			if condition.Type == "Ready" && condition.Status == metav1.ConditionTrue {
				readyCondition = true
			}
			if condition.Type == "ProvisioningSucceeded" && condition.Status == metav1.ConditionTrue {
				provisioningSucceeded = true
			}
		}
		g.Expect(readyCondition).To(BeTrue())
		g.Expect(provisioningSucceeded).To(BeTrue())

		for deploymentName, replicas := range map[string]int32{
			restDeploymentName:  expectedRestReplicas,
			e.flightServiceName: expectedFlightReplicas,
		} {
			deployment := &appsv1.Deployment{}
			deploymentKey := types.NamespacedName{Name: deploymentName, Namespace: applicationNamespace}
			if err := e.k8sClient.Get(e.ctx, deploymentKey, deployment); err != nil {
				g.Expect(err).NotTo(HaveOccurred())
				continue
			}
			g.Expect(deployment.Spec.Replicas).NotTo(BeNil())
			if deployment.Spec.Replicas != nil {
				g.Expect(*deployment.Spec.Replicas).To(Equal(replicas))
			}
			g.Expect(deployment.Status.ReadyReplicas).To(Equal(replicas))
		}
	}, 5*time.Minute, 2*time.Second).Should(Succeed())
}
