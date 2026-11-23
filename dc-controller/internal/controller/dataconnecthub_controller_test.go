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
	"fmt"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	componentsplatformv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/v1alpha1"
)

var _ = Describe("DataConnectHub Controller", func() {
	const (
		moduleName = "default-dataconnecthub"
		appNS      = "default"
		version    = "3.7.0"
	)

	ctx := context.Background()
	key := types.NamespacedName{Name: moduleName}

	newReconciler := func() *DataConnectHubReconciler {
		return &DataConnectHubReconciler{Client: k8sClient, Scheme: k8sClient.Scheme()}
	}

	deleteModule := func() {
		hub := &componentsplatformv1alpha1.DataConnectHub{}
		if err := k8sClient.Get(ctx, key, hub); err == nil {
			if !hub.DeletionTimestamp.IsZero() {
			} else {
				Expect(k8sClient.Delete(ctx, hub)).To(Succeed())
			}
			Eventually(func() error {
				_, reconcileErr := newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
				if reconcileErr != nil {
					return reconcileErr
				}
				current := &componentsplatformv1alpha1.DataConnectHub{}
				getErr := k8sClient.Get(ctx, key, current)
				if getErr != nil {
					return client.IgnoreNotFound(getErr)
				}
				return fmt.Errorf("DataConnectHub finalizer is still pending")
			}).Should(Succeed())
		}
	}

	AfterEach(func() {
		deleteModule()
		Eventually(func() error {
			service := &dchv1alpha1.DataConnectService{}
			err := k8sClient.Get(ctx, types.NamespacedName{Name: dataConnectServiceName, Namespace: appNS}, service)
			if err != nil {
				return client.IgnoreNotFound(err)
			}
			return k8sClient.Delete(ctx, service)
		}).Should(Succeed())
		platformCM := &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: modulePlatformConfigName, Namespace: appNS}}
		Expect(client.IgnoreNotFound(k8sClient.Delete(ctx, platformCM))).To(Succeed())
	})

	It("creates an owned namespaced application CR and projects platform configuration", func() {
		platformConfig := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{Name: modulePlatformConfigName, Namespace: appNS},
			Data:       map[string]string{platformVersionKey: version},
		}
		Expect(k8sClient.Create(ctx, platformConfig)).To(Succeed())

		hub := &componentsplatformv1alpha1.DataConnectHub{
			ObjectMeta: metav1.ObjectMeta{Name: moduleName},
			Spec: componentsplatformv1alpha1.DataConnectHubSpec{
				ApplicationsNamespace: appNS,
				Distribution:          dchv1alpha1.DistributionStatus{Name: distributionOpenDataHub, Version: version},
				Gateway:               &dchv1alpha1.Gateway{Name: "odh-gateway", Namespace: "opendatahub"},
				TokenReviewAudiences:  []string{"https://issuer.example"},
			},
		}
		Expect(k8sClient.Create(ctx, hub)).To(Succeed())

		_, err := newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())

		service := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, types.NamespacedName{Name: dataConnectServiceName, Namespace: appNS}, service)).To(Succeed())
		Expect(service.Spec.Gateway).To(Equal(hub.Spec.Gateway))
		Expect(service.Spec.TokenReviewAudiences).To(Equal(hub.Spec.TokenReviewAudiences))
		owner := metav1.GetControllerOf(service)
		Expect(owner).NotTo(BeNil())
		Expect(owner.UID).To(Equal(hub.UID))

		config := &corev1.ConfigMap{}
		Expect(k8sClient.Get(ctx, types.NamespacedName{Name: modulePlatformConfigName, Namespace: appNS}, config)).To(Succeed())
		Expect(config.Data).To(HaveKeyWithValue(distributionNameKey, distributionOpenDataHub))
		Expect(config.Data).To(HaveKeyWithValue(distributionVersionKey, version))
		Expect(config.Data).To(HaveKeyWithValue(platformVersionKey, version))
		Expect(config.Data).To(HaveKeyWithValue("gateway.name", "odh-gateway"))
		Expect(config.Data).To(HaveKeyWithValue("auth.tokenReviewAudiences", "https://issuer.example"))

		Expect(k8sClient.Get(ctx, key, hub)).To(Succeed())
		Expect(hub.Status.ObservedGeneration).To(Equal(hub.Generation))
		Expect(findCondition(hub.Status.Conditions, conditionReady).Status).To(Equal(metav1.ConditionFalse))
	})

	It("does not adopt an independently created application CR", func() {
		service := &dchv1alpha1.DataConnectService{
			ObjectMeta: metav1.ObjectMeta{Name: dataConnectServiceName, Namespace: appNS},
		}
		Expect(k8sClient.Create(ctx, service)).To(Succeed())
		hub := &componentsplatformv1alpha1.DataConnectHub{
			ObjectMeta: metav1.ObjectMeta{Name: moduleName},
			Spec:       componentsplatformv1alpha1.DataConnectHubSpec{ApplicationsNamespace: appNS},
		}
		Expect(k8sClient.Create(ctx, hub)).To(Succeed())

		_, err := newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())

		current := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, types.NamespacedName{Name: dataConnectServiceName, Namespace: appNS}, current)).To(Succeed())
		Expect(metav1.GetControllerOf(current)).To(BeNil())
		Expect(findConditionByTypeEventually(key, conditionDegraded).Reason).To(Equal("OwnershipConflict"))
	})

	It("mirrors child readiness, degradation, and release status", func() {
		platformConfig := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{Name: modulePlatformConfigName, Namespace: appNS},
			Data:       map[string]string{platformVersionKey: version},
		}
		Expect(k8sClient.Create(ctx, platformConfig)).To(Succeed())
		hub := &componentsplatformv1alpha1.DataConnectHub{
			ObjectMeta: metav1.ObjectMeta{Name: moduleName},
			Spec: componentsplatformv1alpha1.DataConnectHubSpec{
				ApplicationsNamespace: appNS,
				Distribution:          dchv1alpha1.DistributionStatus{Name: distributionOpenDataHub, Version: version},
			},
		}
		Expect(k8sClient.Create(ctx, hub)).To(Succeed())
		_, err := newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())

		service := &dchv1alpha1.DataConnectService{}
		serviceKey := types.NamespacedName{Name: dataConnectServiceName, Namespace: appNS}
		Expect(k8sClient.Get(ctx, serviceKey, service)).To(Succeed())
		service.Status.ObservedGeneration = 0
		service.Status.Distribution = dchv1alpha1.DistributionStatus{Name: distributionOpenDataHub, Version: version}
		service.Status.Releases = []dchv1alpha1.ReleaseStatus{{Name: "platform", Version: version}}
		service.Status.Conditions = []metav1.Condition{
			{Type: conditionReady, Status: metav1.ConditionTrue, ObservedGeneration: 0, LastTransitionTime: metav1.Now(), Reason: conditionReady, Message: "ready"},
			{Type: conditionProvisioningSucceeded, Status: metav1.ConditionTrue, ObservedGeneration: 0, LastTransitionTime: metav1.Now(), Reason: "Provisioned", Message: "provisioned"},
			{Type: conditionDegraded, Status: metav1.ConditionFalse, ObservedGeneration: 0, LastTransitionTime: metav1.Now(), Reason: "Reconciled", Message: "healthy"},
		}
		Expect(k8sClient.Status().Update(ctx, service)).To(Succeed())
		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, key, hub)).To(Succeed())
		Expect(findCondition(hub.Status.Conditions, conditionReady).Status).To(Equal(metav1.ConditionFalse))
		Expect(findCondition(hub.Status.Conditions, conditionReady).Reason).To(Equal("WaitingForDataConnectService"))

		service.Status.ObservedGeneration = service.Generation
		for i := range service.Status.Conditions {
			service.Status.Conditions[i].ObservedGeneration = service.Generation
		}
		Expect(k8sClient.Status().Update(ctx, service)).To(Succeed())
		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, key, hub)).To(Succeed())
		Expect(findCondition(hub.Status.Conditions, conditionReady).Status).To(Equal(metav1.ConditionTrue))
		Expect(findCondition(hub.Status.Conditions, conditionDegraded).Status).To(Equal(metav1.ConditionFalse))
		Expect(hub.Status.Releases).To(ContainElement(dchv1alpha1.ReleaseStatus{Name: "platform", Version: version}))

		service.Status.Conditions[0].Status = metav1.ConditionFalse
		service.Status.Conditions[0].Reason = "GatewayUnavailable"
		service.Status.Conditions[0].Message = "gateway unavailable"
		service.Status.Conditions[2].Status = metav1.ConditionTrue
		service.Status.Conditions[2].Reason = "GatewayUnavailable"
		service.Status.Conditions[2].Message = "gateway unavailable"
		Expect(k8sClient.Status().Update(ctx, service)).To(Succeed())
		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		Expect(k8sClient.Get(ctx, key, hub)).To(Succeed())
		Expect(findCondition(hub.Status.Conditions, conditionReady).Status).To(Equal(metav1.ConditionFalse))
		Expect(findCondition(hub.Status.Conditions, conditionDegraded).Status).To(Equal(metav1.ConditionTrue))
	})

	It("cleans up only the application CR owned by the module", func() {
		platformConfig := &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{Name: modulePlatformConfigName, Namespace: appNS},
			Data:       map[string]string{platformVersionKey: version},
		}
		Expect(k8sClient.Create(ctx, platformConfig)).To(Succeed())
		hub := &componentsplatformv1alpha1.DataConnectHub{
			ObjectMeta: metav1.ObjectMeta{Name: moduleName},
			Spec:       componentsplatformv1alpha1.DataConnectHubSpec{ApplicationsNamespace: appNS},
		}
		Expect(k8sClient.Create(ctx, hub)).To(Succeed())
		_, err := newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		service := &dchv1alpha1.DataConnectService{}
		serviceKey := types.NamespacedName{Name: dataConnectServiceName, Namespace: appNS}
		Expect(k8sClient.Get(ctx, serviceKey, service)).To(Succeed())
		controllerutil.AddFinalizer(service, finalizerName)
		Expect(k8sClient.Update(ctx, service)).To(Succeed())
		Expect(k8sClient.Delete(ctx, hub)).To(Succeed())

		_, err = newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		Expect(k8sClient.Get(ctx, key, hub)).To(Succeed(), "module cleanup must wait for the child finalizer")
		Expect(controllerutil.ContainsFinalizer(hub, dataConnectHubFinalizer)).To(BeTrue())
		Expect(k8sClient.Get(ctx, serviceKey, service)).To(Succeed())
		Expect(service.DeletionTimestamp.IsZero()).To(BeFalse())
		Expect(controllerutil.ContainsFinalizer(service, finalizerName)).To(BeTrue())
		controllerutil.RemoveFinalizer(service, finalizerName)
		Expect(k8sClient.Update(ctx, service)).To(Succeed())

		Eventually(func() error {
			_, reconcileErr := newReconciler().Reconcile(ctx, reconcile.Request{NamespacedName: key})
			if reconcileErr != nil {
				return reconcileErr
			}
			currentHub := &componentsplatformv1alpha1.DataConnectHub{}
			getErr := k8sClient.Get(ctx, key, currentHub)
			if getErr != nil {
				return client.IgnoreNotFound(getErr)
			}
			return fmt.Errorf("DataConnectHub finalizer is still pending")
		}).Should(Succeed())
	})

	It("finishes cleanup in the old namespace before provisioning a new namespace", func() {
		hub := &componentsplatformv1alpha1.DataConnectHub{
			ObjectMeta: metav1.ObjectMeta{Name: moduleName},
			Spec:       componentsplatformv1alpha1.DataConnectHubSpec{ApplicationsNamespace: "new-applications"},
		}
		Expect(k8sClient.Create(ctx, hub)).To(Succeed())
		Expect(k8sClient.Get(ctx, key, hub)).To(Succeed())

		oldService := &dchv1alpha1.DataConnectService{
			ObjectMeta: metav1.ObjectMeta{Name: dataConnectServiceName, Namespace: appNS},
		}
		reconciler := newReconciler()
		Expect(controllerutil.SetControllerReference(hub, oldService, reconciler.Scheme)).To(Succeed())
		Expect(k8sClient.Create(ctx, oldService)).To(Succeed())

		_, err := reconciler.Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())
		_, err = reconciler.Reconcile(ctx, reconcile.Request{NamespacedName: key})
		Expect(err).NotTo(HaveOccurred())

		current := &dchv1alpha1.DataConnectService{}
		err = k8sClient.Get(ctx, types.NamespacedName{Name: dataConnectServiceName, Namespace: appNS}, current)
		Expect(client.IgnoreNotFound(err)).To(Succeed())
		Expect(k8sClient.Get(ctx, key, hub)).To(Succeed())
		Expect(findCondition(hub.Status.Conditions, conditionReady).Reason).To(Equal("CleaningPreviousNamespace"))
	})
})

func findConditionByTypeEventually(key types.NamespacedName, conditionType string) *metav1.Condition {
	var condition *metav1.Condition
	Eventually(func() bool {
		hub := &componentsplatformv1alpha1.DataConnectHub{}
		if err := k8sClient.Get(ctx, key, hub); err != nil {
			return false
		}
		condition = findCondition(hub.Status.Conditions, conditionType)
		return condition != nil
	}).Should(BeTrue())
	return condition
}
