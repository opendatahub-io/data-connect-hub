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
	"slices"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	networkingv1 "k8s.io/api/networking/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	"k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/resource"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

var _ = Describe("DataConnectService Controller", func() {
	const (
		resourceName    = "default-dataconnectservice"
		targetNamespace = "default"
		testRestImage   = "quay.io/opendatahub/odh-data-connect-hub-rest:odh-stable"
		testFlightImage = "quay.io/opendatahub/odh-data-connect-hub-flight:odh-stable"

		// Kustomize adds this prefix to all resource names.
		np = "dch-"
	)

	ctx := context.Background()

	crKey := types.NamespacedName{Name: resourceName, Namespace: targetNamespace}

	manifestsPath := filepath.Join("..", "..", "..", "config")

	reconciler := func() *DataConnectServiceReconciler {
		return &DataConnectServiceReconciler{
			Client:             k8sClient,
			Scheme:             k8sClient.Scheme(),
			ManifestsPath:      manifestsPath,
			RestImage:          testRestImage,
			FlightImage:        testFlightImage,
			KubeRbacProxyImage: "quay.io/opendatahub/odh-kube-rbac-proxy:odh-stable",
		}
	}

	findContainer := func(deploy *appsv1.Deployment, name string) *corev1.Container {
		for i := range deploy.Spec.Template.Spec.Containers {
			if deploy.Spec.Template.Spec.Containers[i].Name == name {
				return &deploy.Spec.Template.Spec.Containers[i]
			}
		}
		return nil
	}

	cleanupOperatorResources := func() {
		for _, name := range []string{np + nameRestService, np + nameFlightService} {
			_ = k8sClient.Delete(ctx, &appsv1.Deployment{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
			_ = k8sClient.Delete(ctx, &corev1.Service{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
			_ = k8sClient.Delete(ctx, &networkingv1.NetworkPolicy{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
		}
		for _, name := range []string{
			np + nameRestService + "-config",
			np + nameFlightService + "-config",
			np + nameRestService + "-kube-rbac-proxy-config",
		} {
			_ = k8sClient.Delete(ctx, &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
		}
		for _, name := range []string{np + nameDataConnectHub + "-sa", np + nameFlightService + "-sa"} {
			_ = k8sClient.Delete(ctx, &corev1.ServiceAccount{ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: targetNamespace}})
		}
		_ = k8sClient.Delete(ctx, &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: nameDatabaseConfig, Namespace: targetNamespace}})
		for _, name := range []string{np + "kube-rbac-proxy-auth-review", np + "read", np + "read-write", np + "admin"} {
			_ = k8sClient.Delete(ctx, &rbacv1.ClusterRole{ObjectMeta: metav1.ObjectMeta{Name: name}})
		}
		for _, name := range []string{np + "kube-rbac-proxy-auth-review", np + "flight-auth-delegator"} {
			_ = k8sClient.Delete(ctx, &rbacv1.ClusterRoleBinding{ObjectMeta: metav1.ObjectMeta{Name: name}})
		}
		_ = k8sClient.Delete(ctx, &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: platformConfigName, Namespace: targetNamespace}})
	}

	deleteCR := func() {
		cr := &dchv1alpha1.DataConnectService{}
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

			cr := &dchv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			if cr.Status.Phase == conditionTypeReady {
				return
			}

			for _, name := range []string{np + nameRestService, np + nameFlightService} {
				deploy := &appsv1.Deployment{}
				if err := k8sClient.Get(ctx, types.NamespacedName{Name: name, Namespace: targetNamespace}, deploy); err == nil {
					simulateDeploymentReady(name)
				}
			}

			if result.RequeueAfter == 0 {
				break
			}
		}

		cr := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal(conditionTypeReady))
	}

	createDatabaseSecret := func() {
		secret := &corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{
				Name:      nameDatabaseConfig,
				Namespace: targetNamespace,
			},
			StringData: map[string]string{
				"DATABASE_URL":       "postgresql://dch:testpass@postgres:5432/dataconnecthub",
				"secret-config.toml": "[database]\nurl = \"postgresql://dch:testpass@postgres:5432/dataconnecthub\"\n",
			},
		}
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())
	}

	Context("When reconciling with default spec", func() {
		BeforeEach(func() {
			createDatabaseSecret()
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: targetNamespace,
				},
				Spec: dchv1alpha1.DataConnectServiceSpec{},
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
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, restDeploy)).To(Succeed())
			restContainer := findContainer(restDeploy, nameRestService)
			Expect(restContainer).NotTo(BeNil())
			Expect(restContainer.Image).To(Equal(testRestImage))
			Expect(*restDeploy.Spec.Replicas).To(Equal(int32(1)))

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameFlightService, Namespace: targetNamespace}, flightDeploy)).To(Succeed())
			Expect(flightDeploy.Spec.Template.Spec.Containers[0].Image).To(Equal(testFlightImage))
		})

		It("should create services for rest and flight", func() {
			reconcileUntilReady()

			restSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, restSvc)).To(Succeed())
			Expect(restSvc.Spec.Ports[0].Port).To(Equal(int32(8443)))

			flightSvc := &corev1.Service{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameFlightService, Namespace: targetNamespace}, flightSvc)).To(Succeed())
			Expect(flightSvc.Spec.Ports[0].Port).To(Equal(int32(8443)))
		})

		It("should set PlatformObject status fields", func() {
			reconcileUntilReady()

			cr := &dchv1alpha1.DataConnectService{}
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

			cr := &dchv1alpha1.DataConnectService{}
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

			for _, name := range []string{np + nameRestService, np + nameFlightService} {
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

		It("should add managed-by label to created resources", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())
			Expect(deploy.Labels).To(HaveKeyWithValue("dataconnecthub.opendatahub.io/managed-by", "dataconnectservice"))
		})
	})

	Context("When reconciling with service overrides", func() {
		BeforeEach(func() {
			createDatabaseSecret()
			customImage := "custom-rest:v2"
			customReplicas := int32(3)
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: targetNamespace,
				},
				Spec: dchv1alpha1.DataConnectServiceSpec{
					RestService: &dchv1alpha1.ServiceOverrides{
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
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())
			restContainer := findContainer(deploy, nameRestService)
			Expect(restContainer).NotTo(BeNil())
			Expect(restContainer.Image).To(Equal("custom-rest:v2"))
			Expect(*deploy.Spec.Replicas).To(Equal(int32(3)))
		})

		It("should apply resource overrides", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())
			restContainer := findContainer(deploy, nameRestService)
			Expect(restContainer).NotTo(BeNil())
			Expect(restContainer.Resources.Requests.Cpu().String()).To(Equal("200m"))
		})

		It("should add custom env vars", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())

			envNames := make(map[string]string)
			restContainer := findContainer(deploy, nameRestService)
			Expect(restContainer).NotTo(BeNil())
			for _, e := range restContainer.Env {
				envNames[e.Name] = e.Value
			}
			Expect(envNames).To(HaveKeyWithValue("CUSTOM_VAR", "custom-value"))
		})
	})

	Context("When imagePullSecrets are specified", func() {
		BeforeEach(func() {
			createDatabaseSecret()
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: targetNamespace,
				},
				Spec: dchv1alpha1.DataConnectServiceSpec{
					RestService: &dchv1alpha1.ServiceOverrides{
						ImagePullSecrets: []corev1.LocalObjectReference{
							{Name: "my-registry-secret"},
						},
					},
					FlightService: &dchv1alpha1.ServiceOverrides{
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
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, restDeploy)).To(Succeed())
			Expect(restDeploy.Spec.Template.Spec.ImagePullSecrets).To(HaveLen(1))
			Expect(restDeploy.Spec.Template.Spec.ImagePullSecrets[0].Name).To(Equal("my-registry-secret"))

			flightDeploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameFlightService, Namespace: targetNamespace}, flightDeploy)).To(Succeed())
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets).To(HaveLen(2))
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets[0].Name).To(Equal("flight-pull-secret"))
			Expect(flightDeploy.Spec.Template.Spec.ImagePullSecrets[1].Name).To(Equal("shared-secret"))
		})
	})

	Context("When tokenReviewAudiences is specified", func() {
		BeforeEach(func() {
			createDatabaseSecret()
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: targetNamespace,
				},
				Spec: dchv1alpha1.DataConnectServiceSpec{
					TokenReviewAudiences: []string{
						"https://rh-oidc.s3.us-east-1.amazonaws.com/test-cluster-id",
					},
				},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should patch flight-service configmap with audiences", func() {
			reconcileUntilReady()

			cm := &corev1.ConfigMap{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameFlightService + "-config", Namespace: targetNamespace}, cm)).To(Succeed())
			toml := cm.Data["config.toml"]
			Expect(toml).To(ContainSubstring(`token_review_audiences = ["https://rh-oidc.s3.us-east-1.amazonaws.com/test-cluster-id"]`))
		})

		It("should add --auth-token-audiences to kube-rbac-proxy", func() {
			reconcileUntilReady()

			deploy := &appsv1.Deployment{}
			Expect(k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, deploy)).To(Succeed())
			proxy := findContainer(deploy, "kube-rbac-proxy")
			Expect(proxy).NotTo(BeNil())

			Expect(slices.Contains(proxy.Args, "--auth-token-audiences=https://rh-oidc.s3.us-east-1.amazonaws.com/test-cluster-id")).
				To(BeTrue(), "expected --auth-token-audiences arg on kube-rbac-proxy")
		})
	})

	Context("When database secret is missing", func() {
		BeforeEach(func() {
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{
					Name:      resourceName,
					Namespace: targetNamespace,
				},
				Spec: dchv1alpha1.DataConnectServiceSpec{},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should set Degraded condition", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: crKey}

			_, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			cr := &dchv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Phase).To(Equal("Error"))

			var degraded *metav1.Condition
			for i := range cr.Status.Conditions {
				if cr.Status.Conditions[i].Type == conditionTypeDegraded {
					degraded = &cr.Status.Conditions[i]
					break
				}
			}
			Expect(degraded).NotTo(BeNil())
			Expect(degraded.Status).To(Equal(metav1.ConditionTrue))
			Expect(degraded.Reason).To(Equal("DatabaseSecretMissing"))
		})

		It("should not create service deployments", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: crKey}

			_, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			restDeploy := &appsv1.Deployment{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: np + nameRestService, Namespace: targetNamespace}, restDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())

			flightDeploy := &appsv1.Deployment{}
			err = k8sClient.Get(ctx, types.NamespacedName{Name: np + nameFlightService, Namespace: targetNamespace}, flightDeploy)
			Expect(errors.IsNotFound(err)).To(BeTrue())
		})
	})

	Context("When CR is deleted", func() {
		It("should not error on reconcile", func() {
			_, err := reconciler().Reconcile(ctx, reconcile.Request{
				NamespacedName: types.NamespacedName{Name: "nonexistent", Namespace: targetNamespace},
			})
			Expect(err).NotTo(HaveOccurred())
		})
	})

	Context("Finalizer behavior", func() {
		BeforeEach(func() {
			createDatabaseSecret()
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName, Namespace: targetNamespace},
				Spec:       dchv1alpha1.DataConnectServiceSpec{},
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

			cr := &dchv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(controllerutil.ContainsFinalizer(cr, finalizerName)).To(BeTrue())
		})

		It("should remove finalizer on deletion", func() {
			r := reconciler()
			_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
			Expect(err).NotTo(HaveOccurred())

			cr := &dchv1alpha1.DataConnectService{}
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
			createDatabaseSecret()
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

			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName, Namespace: targetNamespace},
				Spec:       dchv1alpha1.DataConnectServiceSpec{},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		})

		AfterEach(func() {
			cleanupOperatorResources()
			deleteCR()
		})

		It("should include platform release when platformVersion is set in ConfigMap", func() {
			reconcileUntilReady()

			cr := &dchv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())

			Expect(cr.Status.Releases).To(HaveLen(3))

			var platRelease *dchv1alpha1.ReleaseStatus
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

			cr := &dchv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())

			Expect(cr.Status.Distribution.Name).To(Equal("OpenDataHub"))
			Expect(cr.Status.Distribution.Version).To(Equal("2.20.0"))
		})

		It("should not advance platform version while not Ready", func() {
			r := reconciler()
			req := reconcile.Request{NamespacedName: crKey}

			_, err := r.Reconcile(ctx, req)
			Expect(err).NotTo(HaveOccurred())

			cr := &dchv1alpha1.DataConnectService{}
			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Phase).NotTo(Equal(conditionTypeReady))

			var platRelease *dchv1alpha1.ReleaseStatus
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
			createDatabaseSecret()
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
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName, Namespace: targetNamespace},
				Spec:       dchv1alpha1.DataConnectServiceSpec{},
			}
			Expect(k8sClient.Create(ctx, cr)).To(Succeed())

			reconcileUntilReady()

			Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
			Expect(cr.Status.Gateway).NotTo(BeNil())
			Expect(cr.Status.Gateway.Name).To(Equal("custom-gateway"))
			Expect(cr.Status.Gateway.Namespace).To(Equal("custom-ns"))
		})

		It("should prefer spec.gateway over ConfigMap gateway", func() {
			cr := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{Name: resourceName, Namespace: targetNamespace},
				Spec: dchv1alpha1.DataConnectServiceSpec{
					Gateway: &dchv1alpha1.Gateway{
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
