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
	"testing"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/types"
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
		nsNoCR       = "tenant-rb-nocr"
		nsMapLabel   = "tenant-rb-map-a"
		nsMapPlain   = "tenant-rb-map-b"
		nsUninstall  = "tenant-rb-uninstall"
	)

	allNamespaces := []string{
		nsCreate, nsUnlabelled, nsRemove, nsManual, nsDrift,
		nsNoCR, nsMapLabel, nsMapPlain, nsUninstall,
	}

	ctx := context.Background()

	reconciler := func() *TenantNamespaceReconciler {
		return &TenantNamespaceReconciler{
			Client: k8sClient,
			Scheme: k8sClient.Scheme(),
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

	It("should repair a drifted subject and adopt an unlabelled binding", func() {
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

	It("should do nothing without a live DataConnectService", func() {
		createNamespace(nsNoCR, true)
		reconcileNamespace(nsNoCR)

		expectNoBinding(nsNoCR)
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

	It("should delete tenant RoleBindings on uninstall", func() {
		createCR()
		createNamespace(nsUninstall, true)
		reconcileNamespace(nsUninstall)
		bindingIn(nsUninstall)

		dcs := &DataConnectServiceReconciler{Client: k8sClient, Scheme: k8sClient.Scheme()}
		Expect(dcs.deleteTenantRoleBindings(ctx)).To(Succeed())

		expectNoBinding(nsUninstall)
	})
})

// TestTenantRBACNames pins the kustomize-rendered names the tenant-namespace
// reconciler relies on: the secret-access ClusterRole with its secrets rule,
// and the REST ServiceAccount.
func TestTenantRBACNames(t *testing.T) {
	manifestsPath := filepath.Join("..", "..", "..", "config")
	resources, err := renderKustomization(manifestsPath, filepath.Join(manifestsPath, "base"), nil, nil)
	if err != nil {
		t.Fatalf("rendering base manifests: %v", err)
	}

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
