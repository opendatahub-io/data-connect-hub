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
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	dataconnecthubv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/v1alpha1"
)

var _ = Describe("DataConnectHub Controller", func() {
	const (
		resourceName = "default-dataconnecthub"
		// Cluster-scoped CRs deploy resources into this namespace
		targetNamespace = "default"
	)

	ctx := context.Background()

	crKey := types.NamespacedName{Name: resourceName}

	manifestsPath := filepath.Join("..", "..", "..", "config")

	reconciler := func() *DataConnectHubReconciler {
		return &DataConnectHubReconciler{
			Client:        k8sClient,
			Scheme:        k8sClient.Scheme(),
			ManifestsPath: manifestsPath,
			Namespace:     targetNamespace,
			RestImage:     defaultRestImage,
			FlightImage:   defaultFlightImage,
		}
	}

	cleanupOperatorResources := func() {
		for _, name := range []string{nameRestService, nameFlightService, namePostgres} {
			_ = k8sClient.Delete(ctx, &appsv1.Deployment{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
			_ = k8sClient.Delete(ctx, &corev1.Service{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
		}
		for _, name := range []string{nameRestService + "-config", nameFlightService + "-config"} {
			_ = k8sClient.Delete(ctx, &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
		}
		for _, name := range []string{nameDataConnectHub + "-sa", nameFlightService + "-sa"} {
			_ = k8sClient.Delete(ctx, &corev1.ServiceAccount{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
		}
		_ = k8sClient.Delete(ctx, &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: namePostgresCreds, Namespace: targetNamespace}})
		_ = k8sClient.Delete(ctx, &corev1.PersistentVolumeClaim{ObjectMeta: metav1.ObjectMeta{Name: namePostgres + "-data", Namespace: targetNamespace}})
		for _, name := range []string{nameRestService, nameFlightService, namePostgres} {
			np := &networkingv1.NetworkPolicy{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}}
			_ = k8sClient.Delete(ctx, np)
		}
		_ = k8sClient.Delete(ctx, &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: platformConfigName, Namespace: targetNamespace}})
	}

	deleteCR := func() {
		cr := &dataconnecthubv1alpha1.DataConnectHub{}
		if err := k8sClient.Get(ctx, crKey, cr); err != nil {
			return
		}
		if controllerutil.ContainsFinalizer(cr, finalizerName) {
			controllerutil.RemoveFinalizer(cr, finalizerName)
			_ = k8sClient.Update(ctx, cr)
		}
		_ = k8sClient.Delete(ctx, cr)
	}

	simulateDeploymentReady := func(name string) {
		deploy := &appsv1.Deployment{}
		ExpectWithOffset(1, k8sClient.Get(ctx, types.NamespacedName{Name: name, Namespace: targetNamespace}, deploy)).To(Succeed())
		deploy.Status.Replicas = *deploy.Spec.Replicas
		deploy.Status.ReadyReplicas = *deploy.Spec.Replicas
		deploy.Status.UpdatedReplicas = *deploy.Spec.Replicas
		deploy.Status.ObservedGeneration = deploy.Generation
		ExpectWithOffset(1, k8sClient.Status().Update(ctx, deploy)).To(Succeed())
	}

	reconcileUntilReady := func() {
		r := reconciler()
		req := reconcile.Request{NamespacedName: crKey}

		for range 10 {
			result, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			if cr.Status.Phase == conditionTypeReady {
				return
			}

			for _, name := range []string{namePostgres, nameRestService, nameFlightService} {
				deploy := &appsv1.Deployment{}
				if err := k8sClient.Get(ctx, types.NamespacedName{Name: name, Namespace: targetNamespace}, deploy); err == nil {
					simulateDeploymentReady(name)
				}
			}

			if result.RequeueAfter == 0 {
				break
			}
		}

		cr := &dataconnecthubv1alpha1.DataConnectHub{}
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal(conditionTypeReady))
	}

	Context("When reconciling with default spec", func() {
		BeforeEach(func() {
			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{
					Name: resourceName,
				},
				Spec: dataconnecthubv1alpha1.DataConnectHubSpec{},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should create rest-service and flight-service deployments", func() {
			reconcileUntilReady()

			restDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, restDeploy)).To(Succeed())
			Expect(restDeploy.Spec.Template.Spec.Containers[0].Image).To(Equal(defaultRestImage))
			Expect(*restDeploy.Spec.Replicas).To(Equal(int32(1)))

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: targetNamespace}, flightDeploy)).To(Succeed())
			Expect(flightDeploy.Spec.Template.Spec.Containers[0].Image).To(Equal(defaultFlightImage))
		})

		It("should create services for rest and flight", func() {
			reconcileUntilReady()

			restSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, restSvc)).To(Succeed())
			Expect(restSvc.Spec.Ports[0].Port).To(Equal(int32(8080)))

			flightSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: targetNamespace}, flightSvc)).To(Succeed())
			Expect(flightSvc.Spec.Ports[0].Port).To(Equal(int32(50051)))
		})

		It("should create postgres resources in dev mode by default", func() {
			reconcileUntilReady()

			pgDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: targetNamespace}, pgDeploy)).To(Succeed())

			pgSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: targetNamespace}, pgSvc)).To(Succeed())
			Expect(pgSvc.Spec.Ports[0].Port).To(Equal(int32(5432)))

			pgSecret := &corev1.Secret{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgresCreds, Namespace: targetNamespace}, pgSecret)).To(Succeed())
			Expect(pgSecret.Data).To(HaveKey("secret-config.toml"))

			pgPVC := &corev1.PersistentVolumeClaim{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres + "-data", Namespace: targetNamespace}, pgPVC)).To(Succeed())
		})

		It("should set PlatformObject status fields", func() {
			reconcileUntilReady()

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())

			Expect(cr.Status.ObservedGeneration).To(Equal(cr.Generation))
			Expect(cr.Status.Distribution.Name).To(Equal("Standalone"))
			Expect(cr.Status.Distribution.Version).To(Equal(BuildVersion))
			Expect(cr.Status.Releases).To(HaveLen(2))
			Expect(cr.Status.Releases[0].Name).To(Equal("rest-service"))
			Expect(cr.Status.Releases[1].Name).To(Equal("flight-service"))
		})

		It("should only set Ready when all deployments are available", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: crKey}

			result, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())
			Expect(result.RequeueAfter).To(BeNumerically(">", 0))

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Phase).To(Equal("Progressing"))

			var ready *metav1.Condition
			for i := range cr.Status.Conditions {
				if cr.Status.Conditions[i].Type == "Ready" {
					ready = &cr.Status.Conditions[i]
					break
				}
			}
			Expect(ready).NotTo(BeNil())
			Expect(ready.Status).To(Equal(metav1.ConditionFalse))

			simulateDeploymentReady(namePostgres)
			result, err = r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			for _, name := range []string{nameRestService, nameFlightService} {
				simulateDeploymentReady(name)
			}
			result, err = r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Phase).To(Equal(conditionTypeReady))

			ready = nil
			for i := range cr.Status.Conditions {
				if cr.Status.Conditions[i].Type == "Ready" {
					ready = &cr.Status.Conditions[i]
					break
				}
			}
			Expect(ready).NotTo(BeNil())
			Expect(ready.Status).To(Equal(metav1.ConditionTrue))
		})

		It("should wait for postgres before deploying services", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: crKey}

			result, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())
			Expect(result.RequeueAfter).To(BeNumerically(">", 0))

			pgDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: targetNamespace}, pgDeploy)).To(Succeed())

			restDeploy := &appsv1.Deployment{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, restDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())

			flightDeploy := &appsv1.Deployment{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: targetNamespace}, flightDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())

			simulateDeploymentReady(namePostgres)

			_, err = r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, restDeploy)).To(Succeed())
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: targetNamespace}, flightDeploy)).To(Succeed())
		})

		It("should add managed-by label to created resources", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())
			Expect(deploy.Labels).To(HaveKeyWithValue("components.platform.opendatahub.io/managed-by", "dataconnecthub"))
		})
	})

	Context("When reconciling with service overrides", func() {
		BeforeEach(func() {
			customImage := "custom-rest:v2"
			customReplicas := int32(3)
			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{
					Name: resourceName,
				},
				Spec: dataconnecthubv1alpha1.DataConnectHubSpec{
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
			deleteCR()
		})

		It("should apply image and replicas overrides", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())
			Expect(deploy.Spec.Template.Spec.Containers[0].Image).To(Equal("custom-rest:v2"))
			Expect(*deploy.Spec.Replicas).To(Equal(int32(3)))
		})

		It("should apply resource overrides", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())
			Expect(deploy.Spec.Template.Spec.Containers[0].Resources.Requests.Cpu().String()).To(Equal("200m"))
		})

		It("should add custom env vars", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())

			envNames := make(map[string]string)
			for _, e := range deploy.Spec.Template.Spec.Containers[0].Env {
				envNames[e.Name] = e.Value
			}
			Expect(envNames).To(HaveKeyWithValue("CUSTOM_VAR", "custom-value"))
		})
	})

	Context("When imagePullSecrets are specified", func() {
		BeforeEach(func() {
			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{
					Name: resourceName,
				},
				Spec: dataconnecthubv1alpha1.DataConnectHubSpec{
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
			deleteCR()
		})

		It("should set imagePullSecrets on the deployment pod spec", func() {
			reconcileUntilReady()

			restDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, restDeploy)).To(Succeed())
			Expect(restDeploy.Spec.Template.Spec.ImagePullSecrets).To(HaveLen(1))
			Expect(restDeploy.Spec.Template.Spec.ImagePullSecrets[0].Name).To(Equal("my-registry-secret"))

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: targetNamespace}, flightDeploy)).To(Succeed())
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets).To(HaveLen(2))
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets[0].Name).To(Equal("flight-pull-secret"))
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets[1].Name).To(Equal("shared-secret"))
		})
	})

	Context("When devMode is disabled", func() {
		BeforeEach(func() {
			devMode := false
			extSecret := "my-db-secret"
			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{
					Name: resourceName,
				},
				Spec: dataconnecthubv1alpha1.DataConnectHubSpec{
					DevMode: &devMode,
					Database: &dataconnecthubv1alpha1.DatabaseSpec{
						ExternalSecret: &extSecret,
					},
				},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should not create postgres resources", func() {
			for _, obj := range []client.Object{
				&appsv1.Deployment{ObjectMeta: metav1.ObjectMeta{Name: namePostgres, Namespace: targetNamespace}},
				&corev1.Service{ObjectMeta: metav1.ObjectMeta{Name: namePostgres, Namespace: targetNamespace}},
				&corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: namePostgresCreds, Namespace: targetNamespace}},
				&corev1.PersistentVolumeClaim{ObjectMeta: metav1.ObjectMeta{Name: namePostgres + "-data", Namespace: targetNamespace}},
			} {
				_ = k8sClient.Delete(ctx, obj)
			}

			reconcileUntilReady()

			pgDeploy := &appsv1.Deployment{}
			err := k8sClient.Get(ctx, types.NamespacedName{Name: namePostgres, Namespace: targetNamespace}, pgDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())

			pgSecret := &corev1.Secret{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: namePostgresCreds, Namespace: targetNamespace}, pgSecret)
			Expect(errors.IsNotFound(err)).To(BeTrue())
		})

		It("should still create rest-service and flight-service", func() {
			reconcileUntilReady()

			restDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameRestService, Namespace: targetNamespace}, restDeploy)).To(Succeed())

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nameFlightService, Namespace: targetNamespace}, flightDeploy)).To(Succeed())
		})
	})

	Context("When CR is deleted", func() {
		It("should not error on reconcile", func() {
			_, err := reconciler().Reconcile(ctx, reconcile.Request{
				NamespacedName: types.NamespacedName{Name: "nonexistent"},
			})
			Expect(err).NotTo(HaveOccurred())
		})
	})

	Context("Finalizer behavior", func() {
		BeforeEach(func() {
			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName},
				Spec:       dataconnecthubv1alpha1.DataConnectHubSpec{},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should add finalizer on first reconcile", func() {
			r := reconciler()
			_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
			Expect(err).NotTo(HaveOccurred())

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(controllerutil.ContainsFinalizer(cr, finalizerName)).To(BeTrue())
		})

		It("should remove finalizer on deletion", func() {
			r := reconciler()
			_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
			Expect(err).NotTo(HaveOccurred())

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(k8sClient.Delete(ctx, cr)).To(Succeed())

			_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
			Expect(err).NotTo(HaveOccurred())

			err = k8sClient.Get(ctx, crKey, cr)
			Expect(errors.IsNotFound(err)).To(BeTrue())
		})
	})

	Context("Platform version handshake", func() {
		BeforeEach(func() {
			cm := &corev1.ConfigMap{
				ObjectMeta: metav1.ObjectMeta{
					Name:      platformConfigName,
					Namespace: targetNamespace,
				},
				Data: map[string]string{
					"distribution.name":    "OpenDataHub",
					"distribution.version": "2.20.0",
					"platformVersion":      "2.20.0",
				},
			}
			Expect(k8sClient.Create(ctx, cm)).To(Succeed())

			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName},
				Spec:       dataconnecthubv1alpha1.DataConnectHubSpec{},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should include platform release when platformVersion is set in ConfigMap", func() {
			reconcileUntilReady()

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())

			Expect(cr.Status.Releases).To(HaveLen(3))

			var platRelease *dataconnecthubv1alpha1.ReleaseStatus
			for i := range cr.Status.Releases {
				if cr.Status.Releases[i].Name == releasePlatform {
					platRelease = &cr.Status.Releases[i]
					break
				}
			}
			Expect(platRelease).NotTo(BeNil())
			Expect(platRelease.Version).To(Equal("2.20.0"))
		})

		It("should read distribution from ConfigMap", func() {
			reconcileUntilReady()

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())

			Expect(cr.Status.Distribution.Name).To(Equal("OpenDataHub"))
			Expect(cr.Status.Distribution.Version).To(Equal("2.20.0"))
		})

		It("should not advance platform version while not Ready", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: crKey}

			_, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			cr := &dataconnecthubv1alpha1.DataConnectHub{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Phase).NotTo(Equal(conditionTypeReady))

			var platRelease *dataconnecthubv1alpha1.ReleaseStatus
			for i := range cr.Status.Releases {
				if cr.Status.Releases[i].Name == releasePlatform {
					platRelease = &cr.Status.Releases[i]
					break
				}
			}
			if platRelease != nil {
				Expect(platRelease.Version).To(Equal(""))
			}
		})
	})

	Context("Platform config gateway merge", func() {
		BeforeEach(func() {
			cm := &corev1.ConfigMap{
				ObjectMeta: metav1.ObjectMeta{
					Name:      platformConfigName,
					Namespace: targetNamespace,
				},
				Data: map[string]string{
					"distribution.name":    "Standalone",
					"distribution.version": "0.0.0",
					"gateway.name":         "custom-gateway",
					"gateway.namespace":    "custom-ns",
				},
			}
			Expect(k8sClient.Create(ctx, cm)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should use gateway config from ConfigMap when spec.gateway is not set", func() {
			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName},
				Spec:       dataconnecthubv1alpha1.DataConnectHubSpec{},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())

			reconcileUntilReady()

			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Gateway).NotTo(BeNil())
			Expect(cr.Status.Gateway.Name).To(Equal("custom-gateway"))
			Expect(cr.Status.Gateway.Namespace).To(Equal("custom-ns"))
		})

		It("should prefer spec.gateway over ConfigMap gateway", func() {
			cr := &dataconnecthubv1alpha1.DataConnectHub{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName},
				Spec: dataconnecthubv1alpha1.DataConnectHubSpec{
					Gateway: &dataconnecthubv1alpha1.Gateway{
						Name:      "spec-gateway",
						Namespace: "spec-ns",
					},
				},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())

			reconcileUntilReady()

			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Gateway).NotTo(BeNil())
			Expect(cr.Status.Gateway.Name).To(Equal("spec-gateway"))
			Expect(cr.Status.Gateway.Namespace).To(Equal("spec-ns"))
		})
	})
})
