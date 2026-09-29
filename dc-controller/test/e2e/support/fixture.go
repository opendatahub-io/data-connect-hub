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
	"context"
	"fmt"
	"os/exec"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	clientconfig "sigs.k8s.io/controller-runtime/pkg/client/config"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	"github.com/opendatahub-io/data-connect-hub/dc-controller/test/utils"
)

const (
	// namespace is where the controller manager is deployed.
	namespace = "dc-controller-system"

	serviceAccountName     = "dc-controller-manager"
	metricsServiceName     = "dc-controller-metrics-service"
	metricsRoleBindingName = "dc-controller-metrics-binding"

	applicationNamespace = "dc-controller-e2e"
	postgresName         = "dch-postgres"
	postgresUser         = "dch_user"
	postgresDatabase     = "dch_db"
	dcsName              = "e2e-dataconnectservice"
	restDeploymentName   = "dch-rest-service"
	managerDeployment    = "dc-controller-manager"
	serviceCAConfigMap   = "dch-rest-service-ca"
	serviceCAPath        = "/var/run/secrets/openshift-service-ca"
)

// E2EFixture provisions and owns the shared Kubernetes resources for the E2E suite.
type E2EFixture struct {
	ctx                              context.Context
	k8sClient                        client.Client
	serviceCA                        *testCertificateAuthority
	controllerPodName                string
	dcsKey                           types.NamespacedName
	flightServiceName                string
	originalFlightDeploymentStrategy *appsv1.DeploymentStrategy
}

// NewE2EFixture creates a fixture with the names and context used by the suite.
func NewE2EFixture() *E2EFixture {
	return &E2EFixture{
		ctx:               context.Background(),
		dcsKey:            types.NamespacedName{Name: dcsName, Namespace: applicationNamespace},
		flightServiceName: fmt.Sprintf("dch-%s-flight", dcsName),
	}
}

// Setup installs CRDs, deploys the controller, and provisions the suite dependencies.
func (e *E2EFixture) Setup(managerImage string) {
	By("creating manager namespace")
	cmd := exec.Command("kubectl", "create", "ns", namespace)
	_, err := utils.Run(cmd)
	Expect(err).NotTo(HaveOccurred(), "Failed to create namespace")

	By("labeling the namespace for restricted policy and metrics access")
	cmd = exec.Command("kubectl", "label", "--overwrite", "ns", namespace,
		"pod-security.kubernetes.io/enforce=restricted",
		"metrics=enabled",
	)
	_, err = utils.Run(cmd)
	Expect(err).NotTo(HaveOccurred(), "Failed to label namespace")

	By("installing CRDs")
	cmd = exec.Command("make", "install")
	_, err = utils.Run(cmd)
	Expect(err).NotTo(HaveOccurred(), "Failed to install CRDs")

	By("deploying the controller-manager")
	cmd = exec.Command("make", "deploy", fmt.Sprintf("IMG=%s", managerImage))
	_, err = utils.Run(cmd)
	Expect(err).NotTo(HaveOccurred(), "Failed to deploy the controller-manager")

	By("creating a Kubernetes Go client")
	cfg, err := clientconfig.GetConfig()
	Expect(err).NotTo(HaveOccurred())
	scheme := runtime.NewScheme()
	Expect(appsv1.AddToScheme(scheme)).To(Succeed())
	Expect(corev1.AddToScheme(scheme)).To(Succeed())
	Expect(dchv1alpha1.AddToScheme(scheme)).To(Succeed())
	e.k8sClient, err = client.New(cfg, client.Options{Scheme: scheme})
	Expect(err).NotTo(HaveOccurred())

	By("creating an isolated application namespace")
	Expect(e.k8sClient.Create(e.ctx, &corev1.Namespace{
		ObjectMeta: metav1.ObjectMeta{Name: applicationNamespace},
	})).To(Succeed())

	By("preparing a test CA for REST and Flight service TLS")
	e.serviceCA, err = newTestCertificateAuthority()
	Expect(err).NotTo(HaveOccurred())
	Expect(e.k8sClient.Create(e.ctx, &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Name: serviceCAConfigMap, Namespace: namespace},
		Data:       map[string]string{"service-ca.crt": string(e.serviceCA.certificatePEM)},
	})).To(Succeed())

	By("deploying the DCH PostgreSQL database")
	postgresPassword, err := newTestPassword()
	Expect(err).NotTo(HaveOccurred())
	Expect(installTestPostgres(applicationNamespace, postgresPassword)).To(Succeed())

	By("creating REST and Flight TLS secrets signed by the test CA")
	restTLSSecret, err := e.serviceCA.tlsSecret("rest-service-tls", applicationNamespace,
		restDeploymentName,
		fmt.Sprintf("%s.%s", restDeploymentName, applicationNamespace),
		fmt.Sprintf("%s.%s.svc", restDeploymentName, applicationNamespace),
		fmt.Sprintf("%s.%s.svc.cluster.local", restDeploymentName, applicationNamespace),
	)
	Expect(err).NotTo(HaveOccurred())
	Expect(e.k8sClient.Create(e.ctx, restTLSSecret)).To(Succeed())

	flightTLSSecret, err := e.serviceCA.tlsSecret(fmt.Sprintf("%s-flight-tls", dcsName), applicationNamespace,
		e.flightServiceName,
		fmt.Sprintf("%s.%s", e.flightServiceName, applicationNamespace),
		fmt.Sprintf("%s.%s.svc", e.flightServiceName, applicationNamespace),
		fmt.Sprintf("%s.%s.svc.cluster.local", e.flightServiceName, applicationNamespace),
	)
	Expect(err).NotTo(HaveOccurred())
	Expect(e.k8sClient.Create(e.ctx, flightTLSSecret)).To(Succeed())

	By("creating the database configuration Secret")
	databaseURL := fmt.Sprintf("postgresql://%s:%s@%s.%s.svc:5432/%s",
		postgresUser, postgresPassword, postgresName, applicationNamespace, postgresDatabase)
	Expect(e.k8sClient.Create(e.ctx, &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "dch-database-config", Namespace: applicationNamespace},
		Data: map[string][]byte{
			"secret-config.toml": []byte(fmt.Sprintf("[database]\nurl = %q\n", databaseURL)),
		},
	})).To(Succeed())

	By("mounting the service CA into the controller-manager")
	manager := &appsv1.Deployment{}
	managerKey := types.NamespacedName{Name: managerDeployment, Namespace: namespace}
	Expect(e.k8sClient.Get(e.ctx, managerKey, manager)).To(Succeed())
	base := manager.DeepCopy()
	manager.Spec.Template.Spec.Volumes = append(manager.Spec.Template.Spec.Volumes, corev1.Volume{
		Name: "e2e-service-ca",
		VolumeSource: corev1.VolumeSource{ConfigMap: &corev1.ConfigMapVolumeSource{
			LocalObjectReference: corev1.LocalObjectReference{Name: serviceCAConfigMap},
			Items:                []corev1.KeyToPath{{Key: "service-ca.crt", Path: "service-ca.crt"}},
		}},
	})
	managerContainerFound := false
	for i := range manager.Spec.Template.Spec.Containers {
		if manager.Spec.Template.Spec.Containers[i].Name == "manager" {
			manager.Spec.Template.Spec.Containers[i].VolumeMounts = append(
				manager.Spec.Template.Spec.Containers[i].VolumeMounts,
				corev1.VolumeMount{Name: "e2e-service-ca", MountPath: serviceCAPath, ReadOnly: true},
			)
			managerContainerFound = true
		}
	}
	Expect(managerContainerFound).To(BeTrue())
	Expect(e.k8sClient.Patch(e.ctx, manager, client.MergeFrom(base))).To(Succeed())

	By("waiting for the controller-manager with the CA mount to become available")
	Eventually(func(g Gomega) {
		current := &appsv1.Deployment{}
		g.Expect(e.k8sClient.Get(e.ctx, managerKey, current)).To(Succeed())
		g.Expect(current.Status.ObservedGeneration).To(BeNumerically(">=", current.Generation))
		g.Expect(current.Status.UpdatedReplicas).To(Equal(int32(1)))
		g.Expect(current.Status.AvailableReplicas).To(Equal(int32(1)))
	}, 5*time.Minute, time.Second).Should(Succeed())
}

// Cleanup removes the resources provisioned by Setup.
func (e *E2EFixture) Cleanup() {
	By("cleaning up the curl pod for metrics")
	cmd := exec.Command("kubectl", "delete", "pod", "curl-metrics", "-n", namespace)
	_, _ = utils.Run(cmd)

	By("removing the DCH application namespace")
	cmd = exec.Command("kubectl", "delete", "ns", applicationNamespace,
		"--ignore-not-found=true", "--wait=true", "--timeout=5m")
	_, _ = utils.Run(cmd)

	By("undeploying the controller-manager")
	cmd = exec.Command("make", "undeploy")
	_, _ = utils.Run(cmd)

	By("uninstalling CRDs")
	cmd = exec.Command("make", "uninstall")
	_, _ = utils.Run(cmd)

	By("removing manager namespace")
	cmd = exec.Command("kubectl", "delete", "ns", namespace)
	_, _ = utils.Run(cmd)
}

// DumpDiagnostics writes cluster diagnostics when the current spec has failed.
func (e *E2EFixture) DumpDiagnostics() {
	if CurrentSpecReport().Failed() {
		By("Fetching controller manager pod logs")
		cmd := exec.Command("kubectl", "logs", e.controllerPodName, "-n", namespace)
		controllerLogs, err := utils.Run(cmd)
		if err == nil {
			_, _ = fmt.Fprintf(GinkgoWriter, "Controller logs:\n %s", controllerLogs)
		} else {
			_, _ = fmt.Fprintf(GinkgoWriter, "Failed to get Controller logs: %s", err)
		}

		By("Fetching Kubernetes events")
		cmd = exec.Command("kubectl", "get", "events", "-n", namespace, "--sort-by=.lastTimestamp")
		eventsOutput, err := utils.Run(cmd)
		if err == nil {
			_, _ = fmt.Fprintf(GinkgoWriter, "Kubernetes events:\n%s", eventsOutput)
		} else {
			_, _ = fmt.Fprintf(GinkgoWriter, "Failed to get Kubernetes events: %s", err)
		}

		By("Fetching curl-metrics logs")
		cmd = exec.Command("kubectl", "logs", "curl-metrics", "-n", namespace)
		metricsOutput, err := utils.Run(cmd)
		if err == nil {
			metricsOutput = redactMetricsOutput(metricsOutput)
			_, _ = fmt.Fprintf(GinkgoWriter, "Metrics logs:\n %s", metricsOutput)
		} else {
			_, _ = fmt.Fprintf(GinkgoWriter, "Failed to get curl-metrics logs: %s", err)
		}

		By("Fetching controller manager pod description")
		cmd = exec.Command("kubectl", "describe", "pod", e.controllerPodName, "-n", namespace)
		podDescription, err := utils.Run(cmd)
		if err == nil {
			_, _ = fmt.Fprintln(GinkgoWriter, "Pod description:\n", podDescription)
		} else {
			_, _ = fmt.Fprintln(GinkgoWriter, "Failed to describe controller pod")
		}

		By("Fetching DCH application namespace events and pods")
		cmd = exec.Command("kubectl", "get", "events", "-n", applicationNamespace, "--sort-by=.lastTimestamp")
		applicationEvents, err := utils.Run(cmd)
		if err == nil {
			_, _ = fmt.Fprintf(GinkgoWriter, "Application events:\n%s", applicationEvents)
		}
		cmd = exec.Command("kubectl", "get", "pods", "-n", applicationNamespace, "-o", "wide")
		applicationPods, err := utils.Run(cmd)
		if err == nil {
			_, _ = fmt.Fprintf(GinkgoWriter, "Application pods:\n%s", applicationPods)
		}
	}
}

// AssertControllerReady verifies the deployed controller pod is running and ready.
func (e *E2EFixture) AssertControllerReady() {
	By("validating that the controller-manager pod is running as expected")
	verifyControllerUp := func(g Gomega) {
		By("getting the name of the controller-manager pod")
		podList := &corev1.PodList{}
		err := e.k8sClient.List(e.ctx, podList,
			client.InNamespace(namespace),
			client.MatchingLabels{"control-plane": "manager"},
		)
		g.Expect(err).NotTo(HaveOccurred(), "Failed to retrieve controller-manager pod information")
		podNames := make([]string, 0, len(podList.Items))
		for i := range podList.Items {
			if podList.Items[i].DeletionTimestamp.IsZero() {
				podNames = append(podNames, podList.Items[i].Name)
			}
		}
		if len(podNames) != 1 {
			g.Expect(podNames).To(HaveLen(1), "expected 1 controller pod running")
			return
		}
		e.controllerPodName = podNames[0]
		g.Expect(e.controllerPodName).To(ContainSubstring("manager"))

		By("validating the pod phase and Ready condition")
		pod := &corev1.Pod{}
		podKey := types.NamespacedName{Name: e.controllerPodName, Namespace: namespace}
		if err := e.k8sClient.Get(e.ctx, podKey, pod); err != nil {
			g.Expect(err).NotTo(HaveOccurred())
			return
		}
		g.Expect(pod.Status.Phase).To(Equal(corev1.PodRunning), "Incorrect controller-manager pod phase")

		podReady := false
		for _, condition := range pod.Status.Conditions {
			if condition.Type == corev1.PodReady {
				podReady = condition.Status == corev1.ConditionTrue
				break
			}
		}
		g.Expect(podReady).To(BeTrue(), "Controller-manager pod Ready condition should be True")
	}
	Eventually(verifyControllerUp).Should(Succeed())
}
