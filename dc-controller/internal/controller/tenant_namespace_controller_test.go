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
	"testing"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/events"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/event"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

var _ = Describe("TenantNamespace Controller", func() {
	const (
		crName      = "tenant-rb-dcs"
		crNamespace = "default"

		// envtest runs no namespace controller, so every spec uses its own
		// namespace name: deleted namespaces stay Terminating.
		nsCreate     = "tenant-rb-create"
		nsUnlabelled = "tenant-rb-unlabelled"
		nsRemove     = "tenant-rb-remove"
		nsManual     = "tenant-rb-manual"
		nsDrift      = "tenant-rb-drift"
		nsConflict   = "tenant-rb-conflict"
		nsFinalizer  = "tenant-rb-finalizer"
		nsNoCR       = "tenant-rb-nocr"
		nsMapLabel   = "tenant-rb-map-a"
		nsMapPlain   = "tenant-rb-map-b"
		nsUninstall  = "tenant-rb-uninstall"
		nsLeftover   = "tenant-rb-leftover"
		nsManualNoCR = "tenant-rb-manual-nocr"
		nsRace       = "tenant-rb-race"
	)

	allNamespaces := []string{
		nsCreate, nsUnlabelled, nsRemove, nsManual, nsDrift,
		nsConflict, nsFinalizer, nsNoCR, nsMapLabel, nsMapPlain, nsUninstall,
		nsLeftover, nsManualNoCR, nsRace,
	}

	ctx := context.Background()

	reconciler := func() *TenantNamespaceReconciler {
		return &TenantNamespaceReconciler{
			Client:    k8sClient,
			Scheme:    k8sClient.Scheme(),
			Recorder:  events.NewFakeRecorder(16),
			APIReader: k8sClient,
		}
	}

	reconcileNamespace := func(name string) {
		_, err := reconciler().Reconcile(ctx, reconcile.Request{NamespacedName: types.NamespacedName{Name: name}})
		ExpectWithOffset(1, err).NotTo(HaveOccurred())
	}

	createNamespace := func(name string, labelled bool) {
		ns := &corev1.Namespace{ObjectMeta: metav1.ObjectMeta{Name: name}}
		if labelled {
			ns.Labels = map[string]string{labelTenantNamespace: valueTrue}
		}
		ExpectWithOffset(1, k8sClient.Create(ctx, ns)).To(Succeed())
	}

	createCR := func() {
		cr := &dchv1alpha1.DataConnectService{
			ObjectMeta: metav1.ObjectMeta{Name: crName, Namespace: crNamespace},
		}
		ExpectWithOffset(1, k8sClient.Create(ctx, cr)).To(Succeed())
	}

	deleteCR := func() {
		_ = k8sClient.Delete(ctx, &dchv1alpha1.DataConnectService{
			ObjectMeta: metav1.ObjectMeta{Name: crName, Namespace: crNamespace},
		})
	}

	bindingIn := func(namespace string) *rbacv1.RoleBinding {
		binding := &rbacv1.RoleBinding{}
		key := types.NamespacedName{Name: tenantSecretAccessRoleBindingName(), Namespace: namespace}
		ExpectWithOffset(1, k8sClient.Get(ctx, key, binding)).To(Succeed())
		return binding
	}

	expectNoBinding := func(namespace string) {
		key := types.NamespacedName{Name: tenantSecretAccessRoleBindingName(), Namespace: namespace}
		err := k8sClient.Get(ctx, key, &rbacv1.RoleBinding{})
		ExpectWithOffset(1, apierrors.IsNotFound(err)).To(BeTrue(), "expected no RoleBinding, got err: %v", err)
	}

	AfterEach(func() {
		for _, name := range allNamespaces {
			_ = k8sClient.Delete(ctx, &rbacv1.RoleBinding{
				ObjectMeta: metav1.ObjectMeta{Name: tenantSecretAccessRoleBindingName(), Namespace: name},
			})
			_ = k8sClient.Delete(ctx, &corev1.Namespace{ObjectMeta: metav1.ObjectMeta{Name: name}})
		}
		deleteCR()
	})

	It("should create the secret-access RoleBinding in a labelled tenant namespace", func() {
		createCR()
		createNamespace(nsCreate, true)
		reconcileNamespace(nsCreate)

		binding := bindingIn(nsCreate)
		Expect(binding.RoleRef).To(Equal(rbacv1.RoleRef{
			APIGroup: rbacv1.GroupName,
			Kind:     kindClusterRole,
			Name:     tenantSecretAccessClusterRoleName(),
		}))
		Expect(binding.Subjects).To(Equal([]rbacv1.Subject{{
			Kind:      rbacv1.ServiceAccountKind,
			Name:      restServiceAccountName(),
			Namespace: crNamespace,
		}}))
		Expect(binding.Labels).To(HaveKeyWithValue(labelManagedBy, managedByDCHService))
	})

	It("should not create a RoleBinding in an unlabelled namespace", func() {
		createCR()
		createNamespace(nsUnlabelled, false)
		reconcileNamespace(nsUnlabelled)

		expectNoBinding(nsUnlabelled)
	})

	It("should remove the RoleBinding when the tenant label is removed", func() {
		createCR()
		createNamespace(nsRemove, true)
		reconcileNamespace(nsRemove)
		bindingIn(nsRemove)

		ns := &corev1.Namespace{}
		Expect(k8sClient.Get(ctx, types.NamespacedName{Name: nsRemove}, ns)).To(Succeed())
		delete(ns.Labels, labelTenantNamespace)
		Expect(k8sClient.Update(ctx, ns)).To(Succeed())

		reconcileNamespace(nsRemove)
		expectNoBinding(nsRemove)
	})

	It("should leave a manually created binding alone in an unlabelled namespace", func() {
		createCR()
		createNamespace(nsManual, false)
		manual := &rbacv1.RoleBinding{
			TypeMeta: metav1.TypeMeta{
				APIVersion: rbacv1.SchemeGroupVersion.String(),
				Kind:       "RoleBinding",
			},
			ObjectMeta: metav1.ObjectMeta{
				Name:      tenantSecretAccessRoleBindingName(),
				Namespace: nsManual,
			},
			RoleRef: rbacv1.RoleRef{
				APIGroup: rbacv1.GroupName,
				Kind:     kindClusterRole,
				Name:     tenantSecretAccessClusterRoleName(),
			},
			Subjects: []rbacv1.Subject{{
				Kind:      rbacv1.ServiceAccountKind,
				Name:      restServiceAccountName(),
				Namespace: crNamespace,
			}},
		}
		Expect(k8sClient.Create(ctx, manual)).To(Succeed())

		reconcileNamespace(nsManual)

		Expect(bindingIn(nsManual).ResourceVersion).To(Equal(manual.ResourceVersion))
	})

	It("should repair a drifted subject on a managed binding", func() {
		createCR()
		createNamespace(nsDrift, true)
		reconcileNamespace(nsDrift)

		binding := bindingIn(nsDrift)
		binding.Subjects[0].Namespace = "wrong-namespace"
		Expect(k8sClient.Update(ctx, binding)).To(Succeed())

		reconcileNamespace(nsDrift)

		repaired := bindingIn(nsDrift)
		Expect(repaired.Subjects[0].Namespace).To(Equal(crNamespace))
		Expect(repaired.Labels).To(HaveKeyWithValue(labelManagedBy, managedByDCHService))
	})

	It("should leave an unmanaged binding alone in a labelled tenant namespace", func() {
		createCR()
		createNamespace(nsConflict, true)
		manual := &rbacv1.RoleBinding{
			TypeMeta: metav1.TypeMeta{
				APIVersion: rbacv1.SchemeGroupVersion.String(),
				Kind:       "RoleBinding",
			},
			ObjectMeta: metav1.ObjectMeta{
				Name:      tenantSecretAccessRoleBindingName(),
				Namespace: nsConflict,
			},
			RoleRef: rbacv1.RoleRef{
				APIGroup: rbacv1.GroupName,
				Kind:     kindClusterRole,
				Name:     tenantSecretAccessClusterRoleName(),
			},
			Subjects: []rbacv1.Subject{{
				Kind:      rbacv1.ServiceAccountKind,
				Name:      restServiceAccountName(),
				Namespace: crNamespace,
			}},
		}
		// client.Apply is deprecated in favor of typed apply configurations;
		// this test intentionally applies a full RoleBinding object.
		Expect(k8sClient.Patch(ctx, manual, client.Apply, client.FieldOwner("manual-rolebinding"))).To(Succeed()) //nolint:staticcheck

		fakeRecorder := events.NewFakeRecorder(16)
		_, err := (&TenantNamespaceReconciler{
			Client:    k8sClient,
			Scheme:    k8sClient.Scheme(),
			Recorder:  fakeRecorder,
			APIReader: k8sClient,
		}).Reconcile(ctx, reconcile.Request{NamespacedName: types.NamespacedName{Name: nsConflict}})
		Expect(err).To(MatchError(ContainSubstring("conflicts with tenant onboarding")))

		var event string
		Expect(fakeRecorder.Events).To(Receive(&event))
		Expect(event).To(ContainSubstring("UnmanagedRoleBindingConflict"))

		untouched := bindingIn(nsConflict)
		Expect(untouched.Labels).NotTo(HaveKey(labelManagedBy))
	})

	It("should requeue a replacement blocked by a finalizer and recreate once free", func() {
		createCR()
		createNamespace(nsFinalizer, true)
		drifted := &rbacv1.RoleBinding{
			ObjectMeta: metav1.ObjectMeta{
				Name:       tenantSecretAccessRoleBindingName(),
				Namespace:  nsFinalizer,
				Labels:     map[string]string{labelManagedBy: managedByDCHService},
				Finalizers: []string{"example.com/keep"},
			},
			RoleRef: rbacv1.RoleRef{
				APIGroup: rbacv1.GroupName,
				Kind:     kindClusterRole,
				Name:     "drifted-cluster-role",
			},
			Subjects: []rbacv1.Subject{{
				Kind:      rbacv1.ServiceAccountKind,
				Name:      restServiceAccountName(),
				Namespace: crNamespace,
			}},
		}
		Expect(k8sClient.Create(ctx, drifted)).To(Succeed())

		_, err := reconciler().Reconcile(ctx, reconcile.Request{NamespacedName: types.NamespacedName{Name: nsFinalizer}})
		Expect(err).To(MatchError(ContainSubstring("still terminating")))

		stuck := bindingIn(nsFinalizer)
		Expect(stuck.DeletionTimestamp).NotTo(BeNil())

		stuck.Finalizers = nil
		Expect(k8sClient.Update(ctx, stuck)).To(Succeed())
		expectNoBinding(nsFinalizer)

		reconcileNamespace(nsFinalizer)

		recreated := bindingIn(nsFinalizer)
		Expect(recreated.RoleRef.Name).To(Equal(tenantSecretAccessClusterRoleName()))
		Expect(recreated.Finalizers).To(BeEmpty())
		Expect(recreated.Labels).To(HaveKeyWithValue(labelManagedBy, managedByDCHService))
	})

	It("should not create a RoleBinding without a live DataConnectService", func() {
		createNamespace(nsNoCR, true)
		reconcileNamespace(nsNoCR)

		expectNoBinding(nsNoCR)
	})

	It("should remove a leftover managed binding when no live DataConnectService exists", func() {
		createNamespace(nsLeftover, true)
		leftover := &rbacv1.RoleBinding{
			ObjectMeta: metav1.ObjectMeta{
				Name:      tenantSecretAccessRoleBindingName(),
				Namespace: nsLeftover,
				Labels:    map[string]string{labelManagedBy: managedByDCHService},
			},
			RoleRef: rbacv1.RoleRef{
				APIGroup: rbacv1.GroupName,
				Kind:     kindClusterRole,
				Name:     tenantSecretAccessClusterRoleName(),
			},
			Subjects: []rbacv1.Subject{{
				Kind:      rbacv1.ServiceAccountKind,
				Name:      restServiceAccountName(),
				Namespace: crNamespace,
			}},
		}
		Expect(k8sClient.Create(ctx, leftover)).To(Succeed())

		reconcileNamespace(nsLeftover)

		expectNoBinding(nsLeftover)
	})

	It("should leave an unmanaged binding alone when no live DataConnectService exists", func() {
		createNamespace(nsManualNoCR, true)
		manual := &rbacv1.RoleBinding{
			ObjectMeta: metav1.ObjectMeta{
				Name:      tenantSecretAccessRoleBindingName(),
				Namespace: nsManualNoCR,
			},
			RoleRef: rbacv1.RoleRef{
				APIGroup: rbacv1.GroupName,
				Kind:     kindClusterRole,
				Name:     tenantSecretAccessClusterRoleName(),
			},
			Subjects: []rbacv1.Subject{{
				Kind:      rbacv1.ServiceAccountKind,
				Name:      restServiceAccountName(),
				Namespace: crNamespace,
			}},
		}
		Expect(k8sClient.Create(ctx, manual)).To(Succeed())

		reconcileNamespace(nsManualNoCR)

		Expect(bindingIn(nsManualNoCR).ResourceVersion).To(Equal(manual.ResourceVersion))
	})

	It("should roll back a RoleBinding created while the DataConnectService is being removed", func() {
		createCR()
		createNamespace(nsRace, true)

		// Freeze the cached view while the service is live, then delete
		// the service behind the cache's back: the finalizer's uninstall
		// sweep has already finished by the time the late Create lands.
		frozen := &dchv1alpha1.DataConnectServiceList{}
		Expect(k8sClient.List(ctx, frozen)).To(Succeed())
		deleteCR()
		Eventually(func() bool {
			err := k8sClient.Get(ctx, types.NamespacedName{Name: crName, Namespace: crNamespace}, &dchv1alpha1.DataConnectService{})
			return apierrors.IsNotFound(err)
		}).Should(BeTrue())

		_, err := (&TenantNamespaceReconciler{
			Client:    &staleDCSClient{Client: k8sClient, frozen: frozen},
			Scheme:    k8sClient.Scheme(),
			Recorder:  events.NewFakeRecorder(16),
			APIReader: k8sClient,
		}).Reconcile(ctx, reconcile.Request{NamespacedName: types.NamespacedName{Name: nsRace}})
		Expect(err).NotTo(HaveOccurred())

		expectNoBinding(nsRace)
	})

	It("should map DataConnectService events to labelled namespaces only", func() {
		createNamespace(nsMapLabel, true)
		createNamespace(nsMapPlain, false)

		requests := reconciler().tenantNamespacesToRequests(ctx, &dchv1alpha1.DataConnectService{})
		names := make([]string, 0, len(requests))
		for _, req := range requests {
			names = append(names, req.Name)
		}
		Expect(names).To(ContainElement(nsMapLabel))
		Expect(names).NotTo(ContainElement(nsMapPlain))
	})

	It("should map managed RoleBinding events to their namespace", func() {
		binding := &rbacv1.RoleBinding{
			ObjectMeta: metav1.ObjectMeta{
				Name:      tenantSecretAccessRoleBindingName(),
				Namespace: nsMapLabel,
				Labels:    map[string]string{labelManagedBy: managedByDCHService},
			},
		}

		Expect(roleBindingToNamespaceRequest(ctx, binding)).To(Equal([]reconcile.Request{
			{NamespacedName: types.NamespacedName{Name: nsMapLabel}},
		}))
	})

	It("should pass predicate events only for managed RoleBindings", func() {
		managed := &rbacv1.RoleBinding{
			ObjectMeta: metav1.ObjectMeta{
				Name:      tenantSecretAccessRoleBindingName(),
				Namespace: nsMapLabel,
				Labels:    map[string]string{labelManagedBy: managedByDCHService},
			},
		}
		unmanaged := managed.DeepCopy()
		unmanaged.Labels = nil
		renamed := managed.DeepCopy()
		renamed.Name = "other-rolebinding"

		p := managedRoleBindingPredicate()
		Expect(p.Create(event.TypedCreateEvent[*rbacv1.RoleBinding]{Object: managed})).To(BeTrue())
		Expect(p.Create(event.TypedCreateEvent[*rbacv1.RoleBinding]{Object: unmanaged})).To(BeFalse())
		Expect(p.Create(event.TypedCreateEvent[*rbacv1.RoleBinding]{Object: renamed})).To(BeFalse())
		Expect(p.Delete(event.TypedDeleteEvent[*rbacv1.RoleBinding]{Object: managed})).To(BeTrue())
		Expect(p.Delete(event.TypedDeleteEvent[*rbacv1.RoleBinding]{Object: unmanaged})).To(BeFalse())
		// An update from either side of the managed state passes, so an
		// administrator taking over or releasing the binding is still
		// reconciled.
		Expect(p.Update(event.TypedUpdateEvent[*rbacv1.RoleBinding]{ObjectOld: managed, ObjectNew: unmanaged})).To(BeTrue())
		Expect(p.Update(event.TypedUpdateEvent[*rbacv1.RoleBinding]{ObjectOld: unmanaged, ObjectNew: managed})).To(BeTrue())
		Expect(p.Update(event.TypedUpdateEvent[*rbacv1.RoleBinding]{ObjectOld: unmanaged, ObjectNew: renamed})).To(BeFalse())
	})

	It("should delete tenant RoleBindings on uninstall", func() {
		createCR()
		createNamespace(nsUninstall, true)
		reconcileNamespace(nsUninstall)
		bindingIn(nsUninstall)

		dcs := &DataConnectServiceReconciler{Client: k8sClient, Scheme: k8sClient.Scheme(), APIReader: k8sClient}
		Expect(dcs.deleteTenantRoleBindings(ctx)).To(Succeed())

		expectNoBinding(nsUninstall)
	})
})

// staleDCSClient serves DataConnectService lists from a frozen snapshot
// while delegating every other call, simulating an informer cache that lags
// behind the API server during DataConnectService deletion: the reconciler
// still sees a live service after the finalizer's uninstall sweep finished.
type staleDCSClient struct {
	client.Client
	frozen *dchv1alpha1.DataConnectServiceList
}

func (c *staleDCSClient) List(ctx context.Context, list client.ObjectList, opts ...client.ListOption) error {
	if dcsList, ok := list.(*dchv1alpha1.DataConnectServiceList); ok {
		c.frozen.DeepCopyInto(dcsList)
		return nil
	}
	return c.Client.List(ctx, list, opts...)
}

// TestTenantRBACNames pins the chart-rendered names the tenant-namespace
// reconciler relies on: the secret-access ClusterRole with its secrets rule,
// and the REST ServiceAccount.
func TestTenantRBACNames(t *testing.T) {
	resources := renderChart(t)

	haveClusterRole := false
	haveSecretsRule := false
	haveServiceAccount := false
	for _, res := range resources {
		switch {
		case res.GetKind() == kindClusterRole && res.GetName() == tenantSecretAccessClusterRoleName():
			haveClusterRole = true
			rules, found, err := unstructured.NestedSlice(res.Object, "rules")
			if err != nil || !found {
				t.Errorf("ClusterRole %s has no rules", res.GetName())
				continue
			}
			for _, rule := range rules {
				ruleResources, _, _ := unstructured.NestedStringSlice(rule.(map[string]any), "resources")
				for _, r := range ruleResources {
					if r == "secrets" {
						haveSecretsRule = true
					}
				}
			}
		case res.GetKind() == kindServiceAccount && res.GetName() == restServiceAccountName():
			haveServiceAccount = true
		}
	}
	if !haveClusterRole {
		t.Errorf("rendered manifests contain no ClusterRole %q for the tenant RoleBinding", tenantSecretAccessClusterRoleName())
	}
	if !haveSecretsRule {
		t.Errorf("ClusterRole %s grants no access to secrets", tenantSecretAccessClusterRoleName())
	}
	if !haveServiceAccount {
		t.Errorf("rendered manifests contain no ServiceAccount %q for the tenant RoleBinding subject", restServiceAccountName())
	}
}
