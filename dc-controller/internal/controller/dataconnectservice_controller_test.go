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
	"path/filepath"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	networkingv1 "k8s.io/api/networking/v1"
	"k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/resource"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	dataconnecthubv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/v1alpha1"
)

var _ = Describe("DataConnectService Controller", func() {
	const (
		resourceName      = "test-dcs"
		resourceNamespace = "default"
	)

	ctx := context.Background()

	typeNamespacedName := types.NamespacedName{
		Name:      resourceName,
		Namespace: resourceNamespace,
	}

	manifestsPath := filepath.Join("..", "..", "..", "config")

	reconciler := func() *DataConnectServiceReconciler {
		return &DataConnectServiceReconciler{
			Client:        k8sClient,
			Scheme:        k8sClient.Scheme(),
			ManifestsPath: manifestsPath,
		}
	}

	// cleanupOperatorResources removes all resources the operator creates.
	// envtest has no garbage collector, so owner-referenced resources persist.
	cleanupOperatorResources := func() {
		for _, name := range []string{nameRestService, nameFlightService, namePostgres} {
			_ = k8sClient.Delete(ctx, &appsv1.Deployment{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: resourceNamespace}})
			_ = k8sClient.Delete(ctx, &corev1.Service{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: resourceNamespace}})
		}
		for _, name := range []string{nameRestService + "-config", nameFlightService + "-config"} {
			_ = k8sClient.Delete(ctx, &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: resourceNamespace}})
		}
		for _, name := range []string{nameDataConnectHub + "-sa", nameFlightService + "-sa"} {
			_ = k8sClient.Delete(ctx, &corev1.ServiceAccount{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: resourceNamespace}})
		}
		_ = k8sClient.Delete(ctx, &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: namePostgresCreds, Namespace: resourceNamespace}})
		_ = k8sClient.Delete(ctx, &corev1.PersistentVolumeClaim{ObjectMeta: metav1.ObjectMeta{Name: namePostgres + "-data", Namespace: resourceNamespace}})
		for _, name := range []string{nameRestService, nameFlightService, namePostgres} {
			np := &networkingv1.NetworkPolicy{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: resourceNamespace}}
			_ = k8sClient.Delete(ctx, np)
		}
	}

	// simulateDeploymentReady sets the status fields that isDeploymentReady checks.
	simulateDeploymentReady := func(name string) {
		deploy := &appsv1.Deployment{}
		ExpectWithOffset(1, k8sClient.Get(ctx, types.NamespacedName{Name: name, Namespace: resourceNamespace}, deploy)).To(Succeed())
		deploy.Status.Replicas = *deploy.Spec.Replicas
		deploy.Status.ReadyReplicas = *deploy.Spec.Replicas
		deploy.Status.UpdatedReplicas = *deploy.Spec.Replicas
		deploy.Status.ObservedGeneration = deploy.Generation
		ExpectWithOffset(1, k8sClient.Status().Update(ctx, deploy)).To(Succeed())
	}

	// reconcileUntilReady runs Reconcile in a loop, simulating deployment readiness
	// after resources are created, until the CR reaches the Ready phase.
	reconcileUntilReady := func() {
		r := reconciler()
		req := reconcile.Request{NamespacedName: typeNamespacedName}

		for range 10 {
			result, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			cr := &dataconnecthubv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, typeNamespacedName, cr)).To(Succeed())
			if cr.Status.Phase == "Ready" {
				return
			}

			// Simulate readiness for all deployments that exist
			for _, name := range []string{namePostgres, nameRestService, nameFlightService} {
				deploy := &appsv1.Deployment{}
				if err := k8sClient.Get(ctx, types.NamespacedName{Name: name, Namespace: resourceNamespace}, deploy); err == nil {
					simulateDeploymentReady(name)
				}
			}

			if result.RequeueAfter == 0 {
				break
			}
		}

		cr := &dataconnecthubv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, typeNamespacedName, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Ready"))
	}

	Context("When reconciling with default spec", func() {
		BeforeEach(func() {
			cr := &dataconnecthubv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: resourceNamespace,
				},
				Spec: dataconnecthubv1alpha1.DataConnectServiceSpec{
					Description: "test instance",
				},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			cr := &dataconnecthubv1alpha1.DataConnectService{}
			err := k8sClient.Get(ctx, typeNamespacedName, cr)
			if err == nil {
				Expect(k8sClient.Delete(ctx, cr)).To(Succeed())
			}
		})

		It("should create rest-service and flight-service deployments", func() {
			reconcileUntilReady()

			restDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, restDeploy)).To(Succeed())
			Expect(restDeploy.Spec.Template.Spec.Containers[0].Image).To(Equal(defaultRestImage))
			Expect(*restDeploy.Spec.Replicas).To(Equal(int32(1)))

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: resourceNamespace}, flightDeploy)).To(Succeed())
			Expect(flightDeploy.Spec.Template.Spec.Containers[0].Image).To(Equal(defaultFlightImage))
		})

		It("should create services for rest and flight", func() {
			reconcileUntilReady()

			restSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, restSvc)).To(Succeed())
			Expect(restSvc.Spec.Ports[0].Port).To(Equal(int32(8080)))

			flightSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: resourceNamespace}, flightSvc)).To(Succeed())
			Expect(flightSvc.Spec.Ports[0].Port).To(Equal(int32(50051)))
		})

		It("should create postgres resources in dev mode by default", func() {
			reconcileUntilReady()

			pgDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: resourceNamespace}, pgDeploy)).To(Succeed())

			pgSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: resourceNamespace}, pgSvc)).To(Succeed())
			Expect(pgSvc.Spec.Ports[0].Port).To(Equal(int32(5432)))

			pgSecret := &corev1.Secret{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgresCreds, Namespace: resourceNamespace}, pgSecret)).To(Succeed())
			Expect(pgSecret.Data).To(HaveKey("secret-config.toml"))

			pgPVC := &corev1.PersistentVolumeClaim{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres + "-data", Namespace: resourceNamespace}, pgPVC)).To(Succeed())
		})

		It("should only set Ready when all deployments are available", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: typeNamespacedName}

			// First reconcile: creates postgres, but it's not ready yet
			result, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())
			Expect(result.RequeueAfter).To(BeNumerically(">", 0))

			cr := &dataconnecthubv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, typeNamespacedName, cr)).To(Succeed())
			Expect(cr.Status.Phase).To(Equal("Progressing"))

			var available *metav1.Condition
			for i := range cr.Status.Conditions {
				if cr.Status.Conditions[i].Type == "Available" {
					available = &cr.Status.Conditions[i]
					break
				}
			}
			Expect(available).NotTo(BeNil())
			Expect(available.Status).To(Equal(metav1.ConditionFalse))

			// Now simulate all deployments becoming ready and reconcile again
			simulateDeploymentReady(namePostgres)
			result, err = r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			// Services deployed but not ready yet
			for _, name := range []string{nameRestService, nameFlightService} {
				simulateDeploymentReady(name)
			}
			result, err = r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			Expect(k8sClient.Get(ctx, typeNamespacedName, cr)).To(Succeed())
			Expect(cr.Status.Phase).To(Equal("Ready"))

			available = nil
			for i := range cr.Status.Conditions {
				if cr.Status.Conditions[i].Type == "Available" {
					available = &cr.Status.Conditions[i]
					break
				}
			}
			Expect(available).NotTo(BeNil())
			Expect(available.Status).To(Equal(metav1.ConditionTrue))
		})

		It("should wait for postgres before deploying services", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: typeNamespacedName}

			// First reconcile: creates postgres resources, requeues waiting for postgres
			result, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())
			Expect(result.RequeueAfter).To(BeNumerically(">", 0))

			// Postgres deployment should exist
			pgDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: resourceNamespace}, pgDeploy)).To(Succeed())

			// But rest-service and flight-service should NOT exist yet
			restDeploy := &appsv1.Deployment{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, restDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())

			flightDeploy := &appsv1.Deployment{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: resourceNamespace}, flightDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())

			// Simulate postgres becoming ready
			simulateDeploymentReady(namePostgres)

			// Second reconcile: now services should be created
			_, err = r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, restDeploy)).To(Succeed())
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: resourceNamespace}, flightDeploy)).To(Succeed())
		})
	})

	Context("When reconciling with service overrides", func() {
		BeforeEach(func() {
			customImage := "custom-rest:v2"
			customReplicas := int32(3)
			cr := &dataconnecthubv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: resourceNamespace,
				},
				Spec: dataconnecthubv1alpha1.DataConnectServiceSpec{
					RestService: &dataconnecthubv1alpha1.ServiceOverrides{
						Image:    &customImage,
						Replicas: &customReplicas,
						Resources: &corev1.ResourceRequirements{
							Requests: corev1.ResourceList{
								corev1.ResourceCPU:    resource.MustParse("200m"),
								corev1.ResourceMemory: resource.MustParse("512Mi"),
							},
							Limits: corev1.ResourceList{
								corev1.ResourceCPU:    resource.MustParse("2"),
								corev1.ResourceMemory: resource.MustParse("1Gi"),
							},
						},
						Env: []corev1.EnvVar{
							{Name: "CUSTOM_VAR", Value: "custom-value"},
						},
					},
				},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			cr := &dataconnecthubv1alpha1.DataConnectService{}
			err := k8sClient.Get(ctx, typeNamespacedName, cr)
			if err == nil {
				Expect(k8sClient.Delete(ctx, cr)).To(Succeed())
			}
		})

		It("should apply image and replicas overrides", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, deploy)).To(Succeed())
			Expect(deploy.Spec.Template.Spec.Containers[0].Image).To(Equal("custom-rest:v2"))
			Expect(*deploy.Spec.Replicas).To(Equal(int32(3)))
		})

		It("should apply resource overrides", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, deploy)).To(Succeed())
			Expect(deploy.Spec.Template.Spec.Containers[0].Resources.Requests.Cpu().String()).To(Equal("200m"))
		})

		It("should add custom env vars", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, deploy)).To(Succeed())

			envNames := make(map[string]string)
			for _, e := range deploy.Spec.Template.Spec.Containers[0].Env {
				envNames[e.Name] = e.Value
			}
			Expect(envNames).To(HaveKeyWithValue("CUSTOM_VAR", "custom-value"))
		})
	})

	Context("When imagePullSecrets are specified", func() {
		BeforeEach(func() {
			cr := &dataconnecthubv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: resourceNamespace,
				},
				Spec: dataconnecthubv1alpha1.DataConnectServiceSpec{
					RestService: &dataconnecthubv1alpha1.ServiceOverrides{
						ImagePullSecrets: []corev1.LocalObjectReference{
							{Name: "my-registry-secret"},
						},
					},
					FlightService: &dataconnecthubv1alpha1.ServiceOverrides{
						ImagePullSecrets: []corev1.LocalObjectReference{
							{Name: "flight-pull-secret"},
							{Name: "shared-secret"},
						},
					},
				},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			cr := &dataconnecthubv1alpha1.DataConnectService{}
			err := k8sClient.Get(ctx, typeNamespacedName, cr)
			if err == nil {
				Expect(k8sClient.Delete(ctx, cr)).To(Succeed())
			}
		})

		It("should set imagePullSecrets on the deployment pod spec", func() {
			reconcileUntilReady()

			restDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, restDeploy)).To(Succeed())
			Expect(restDeploy.Spec.Template.Spec.ImagePullSecrets).To(HaveLen(1))
			Expect(restDeploy.Spec.Template.Spec.ImagePullSecrets[0].Name).To(Equal("my-registry-secret"))

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: resourceNamespace}, flightDeploy)).To(Succeed())
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets).To(HaveLen(2))
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets[0].Name).To(Equal("flight-pull-secret"))
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets[1].Name).To(Equal("shared-secret"))
		})
	})

	Context("When devMode is disabled", func() {
		BeforeEach(func() {
			devMode := false
			cr := &dataconnecthubv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: resourceNamespace,
				},
				Spec: dataconnecthubv1alpha1.DataConnectServiceSpec{
					Database: &dataconnecthubv1alpha1.DatabaseSpec{
						DevMode: &devMode,
					},
				},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			cr := &dataconnecthubv1alpha1.DataConnectService{}
			err := k8sClient.Get(ctx, typeNamespacedName, cr)
			if err == nil {
				Expect(k8sClient.Delete(ctx, cr)).To(Succeed())
			}
		})

		It("should not create postgres resources", func() {
			for _, obj := range []client.Object{
				&appsv1.Deployment{ObjectMeta: metav1.ObjectMeta{Name: namePostgres, Namespace: resourceNamespace}},
				&corev1.Service{ObjectMeta: metav1.ObjectMeta{Name: namePostgres, Namespace: resourceNamespace}},
				&corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: namePostgresCreds, Namespace: resourceNamespace}},
				&corev1.PersistentVolumeClaim{ObjectMeta: metav1.ObjectMeta{Name: namePostgres + "-data", Namespace: resourceNamespace}},
			} {
				_ = k8sClient.Delete(ctx, obj)
			}

			reconcileUntilReady()

			pgDeploy := &appsv1.Deployment{}
			err := k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: resourceNamespace}, pgDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())

			pgSecret := &corev1.Secret{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: namePostgresCreds, Namespace: resourceNamespace}, pgSecret)
			Expect(errors.IsNotFound(err)).To(BeTrue())
		})

		It("should still create rest-service and flight-service", func() {
			reconcileUntilReady()

			restDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: resourceNamespace}, restDeploy)).To(Succeed())

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: resourceNamespace}, flightDeploy)).To(Succeed())
		})
	})

	Context("When CR is deleted", func() {
		It("should not error on reconcile", func() {
			_, err := reconciler().Reconcile(ctx, reconcile.Request{
				NamespacedName: types.NamespacedName{Name: "nonexistent", Namespace: resourceNamespace},
			})
			Expect(err).NotTo(HaveOccurred())
		})
	})
})
