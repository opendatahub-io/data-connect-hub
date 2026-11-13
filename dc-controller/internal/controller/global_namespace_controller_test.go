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

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

var _ = Describe("GlobalNamespace Controller", Ordered, func() {
	const (
		labelKey   = "test.dataconnecthub.opendatahub.io/global"
		serviceNS  = "gns-dch"
		serviceCR  = "gns-service"
		globalA    = "gns-global-a"
		globalB    = "gns-global-b"
		curatorsRB = "global-curators"
	)
	ctx := context.Background()
	gn := GlobalNamespaceConfig{Enabled: true, LabelKey: labelKey}
	crKey := types.NamespacedName{Name: serviceCR, Namespace: serviceNS}

	var r *GlobalNamespaceReconciler

	reconcileOnce := func() {
		_, err := r.Reconcile(ctx, globalNSReconcileKey)
		Expect(err).NotTo(HaveOccurred())
	}

	setLabel := func(name string, labeled bool) {
		ns := &corev1.Namespace{}
		Expect(k8sClient.Get(ctx, types.NamespacedName{Name: name}, ns)).To(Succeed())
		if ns.Labels == nil {
			ns.Labels = map[string]string{}
		}
		if labeled {
			ns.Labels[labelKey] = name
		} else {
			delete(ns.Labels, labelKey)
		}
		Expect(k8sClient.Update(ctx, ns)).To(Succeed())
	}

	setAuth := func(allowed, admin []string) {
		auth := &unstructured.Unstructured{}
		auth.SetGroupVersionKind(platformAuthGVK)
		err := k8sClient.Get(ctx, types.NamespacedName{Name: platformAuthCRName}, auth)
		if apierrors.IsNotFound(err) {
			auth.SetName(platformAuthCRName)
			Expect(unstructured.SetNestedStringSlice(auth.Object, allowed, "spec", "allowedGroups")).To(Succeed())
			Expect(unstructured.SetNestedStringSlice(auth.Object, admin, "spec", "adminGroups")).To(Succeed())
			Expect(k8sClient.Create(ctx, auth)).To(Succeed())
			return
		}
		Expect(err).NotTo(HaveOccurred())
		Expect(unstructured.SetNestedStringSlice(auth.Object, allowed, "spec", "allowedGroups")).To(Succeed())
		Expect(unstructured.SetNestedStringSlice(auth.Object, admin, "spec", "adminGroups")).To(Succeed())
		Expect(k8sClient.Update(ctx, auth)).To(Succeed())
	}

	deleteAuth := func() {
		auth := &unstructured.Unstructured{}
		auth.SetGroupVersionKind(platformAuthGVK)
		auth.SetName(platformAuthCRName)
		Expect(client.IgnoreNotFound(k8sClient.Delete(ctx, auth))).To(Succeed())
	}

	getRB := func(namespace, name string) (*rbacv1.RoleBinding, error) {
		rb := &rbacv1.RoleBinding{}
		err := k8sClient.Get(ctx, types.NamespacedName{Namespace: namespace, Name: name}, rb)
		return rb, err
	}

	subjectNames := func(rb *rbacv1.RoleBinding) []string {
		names := make([]string, 0, len(rb.Subjects))
		for _, s := range rb.Subjects {
			names = append(names, s.Name)
		}
		return names
	}

	expectManagedRoleBindings := func(namespace string) {
		for _, name := range managedGlobalRoleBindings {
			_, err := getRB(namespace, name)
			Expect(err).NotTo(HaveOccurred(), "RoleBinding %s/%s", namespace, name)
		}
	}

	expectNoManagedRoleBindings := func(namespace string) {
		for _, name := range managedGlobalRoleBindings {
			_, err := getRB(namespace, name)
			Expect(apierrors.IsNotFound(err)).To(BeTrue(), "RoleBinding %s/%s should not exist", namespace, name)
		}
	}

	tenantsFor := func(cfg GlobalNamespaceConfig) []string {
		labeled, err := labeledGlobalNamespaces(ctx, k8sClient, cfg)
		Expect(err).NotTo(HaveOccurred())
		return globalTenants(serviceNS, labeled)
	}

	BeforeAll(func() {
		for _, name := range []string{serviceNS, globalA, globalB} {
			ns := &corev1.Namespace{ObjectMeta: metav1.ObjectMeta{Name: name}}
			Expect(client.IgnoreAlreadyExists(k8sClient.Create(ctx, ns))).To(Succeed())
		}
		Expect(k8sClient.Create(ctx, &dchv1alpha1.DataConnectService{
			ObjectMeta: metav1.ObjectMeta{Name: serviceCR, Namespace: serviceNS},
		})).To(Succeed())
		setAuth([]string{testGroupAuthenticated}, []string{testGroupAdmins})

		r = &GlobalNamespaceReconciler{
			Client:       k8sClient,
			RoleBindings: k8sClient,
			Config:       gn,
		}
	})

	AfterAll(func() {
		for _, name := range []string{globalA, globalB} {
			setLabel(name, false)
		}
		reconcileOnce()
		_ = k8sClient.Delete(ctx, &rbacv1.RoleBinding{ObjectMeta: metav1.ObjectMeta{Name: curatorsRB, Namespace: globalA}})
		cr := &dchv1alpha1.DataConnectService{}
		if err := k8sClient.Get(ctx, crKey, cr); err == nil {
			controllerutil.RemoveFinalizer(cr, finalizerName)
			_ = k8sClient.Update(ctx, cr)
			_ = k8sClient.Delete(ctx, cr)
		}
		deleteAuth()
	})

	It("grants nothing when no namespace is labeled", func() {
		reconcileOnce()
		expectNoManagedRoleBindings(globalA)
	})

	It("grants the Auth groups and DCH service accounts access in the labeled namespace", func() {
		setLabel(globalA, true)
		reconcileOnce()
		expectManagedRoleBindings(globalA)

		read, err := getRB(globalA, roleBindingGlobalRead)
		Expect(err).NotTo(HaveOccurred())
		Expect(read.RoleRef.Name).To(Equal(clusterRoleRead))
		Expect(subjectNames(read)).To(Equal([]string{testGroupAdmins, testGroupAuthenticated}))

		write, err := getRB(globalA, roleBindingGlobalReadWrite)
		Expect(err).NotTo(HaveOccurred())
		Expect(write.RoleRef.Name).To(Equal(clusterRoleReadWrite))
		Expect(subjectNames(write)).To(Equal([]string{testGroupAdmins}))

		restRB, err := getRB(globalA, roleBindingGlobalRestSecretAccess)
		Expect(err).NotTo(HaveOccurred())
		Expect(restRB.Subjects).To(ContainElement(rbacv1.Subject{
			Kind: rbacv1.ServiceAccountKind, Namespace: serviceNS, Name: serviceAccountRestService,
		}))
		flightRB, err := getRB(globalA, roleBindingGlobalFlightSecretAccess)
		Expect(err).NotTo(HaveOccurred())
		Expect(flightRB.Subjects).To(ContainElement(rbacv1.Subject{
			Kind: rbacv1.ServiceAccountKind, Namespace: serviceNS, Name: flightServiceAccountName(serviceCR),
		}))

	})

	It("makes the DataConnectService namespace and the labeled namespaces global connection type tenants", func() {
		Expect(tenantsFor(gn)).To(Equal([]string{serviceNS, globalA}))
		Expect(tenantsFor(GlobalNamespaceConfig{LabelKey: labelKey})).To(Equal([]string{serviceNS}),
			"disabled feature keeps only the DataConnectService namespace")
	})

	It("repairs drift and leaves administrator-created bindings alone", func() {
		Expect(k8sClient.Create(ctx, &rbacv1.RoleBinding{
			ObjectMeta: metav1.ObjectMeta{Name: curatorsRB, Namespace: globalA},
			RoleRef:    rbacv1.RoleRef{APIGroup: rbacv1.GroupName, Kind: kindClusterRole, Name: clusterRoleReadWrite},
			Subjects:   []rbacv1.Subject{{APIGroup: rbacv1.GroupName, Kind: rbacv1.GroupKind, Name: "data-curators"}},
		})).To(Succeed())

		read, err := getRB(globalA, roleBindingGlobalRead)
		Expect(err).NotTo(HaveOccurred())
		Expect(k8sClient.Delete(ctx, read)).To(Succeed())

		write, err := getRB(globalA, roleBindingGlobalReadWrite)
		Expect(err).NotTo(HaveOccurred())
		write.Subjects = append(write.Subjects, rbacv1.Subject{APIGroup: rbacv1.GroupName, Kind: rbacv1.GroupKind, Name: "intruders"})
		Expect(k8sClient.Update(ctx, write)).To(Succeed())

		reconcileOnce()

		read, err = getRB(globalA, roleBindingGlobalRead)
		Expect(err).NotTo(HaveOccurred())
		Expect(subjectNames(read)).To(Equal([]string{testGroupAdmins, testGroupAuthenticated}))
		write, err = getRB(globalA, roleBindingGlobalReadWrite)
		Expect(err).NotTo(HaveOccurred())
		Expect(subjectNames(write)).To(Equal([]string{testGroupAdmins}))

		curators, err := getRB(globalA, curatorsRB)
		Expect(err).NotTo(HaveOccurred())
		Expect(subjectNames(curators)).To(Equal([]string{"data-curators"}))
	})

	It("recreates a managed binding whose roleRef was changed", func() {
		read, err := getRB(globalA, roleBindingGlobalRead)
		Expect(err).NotTo(HaveOccurred())
		Expect(k8sClient.Delete(ctx, read)).To(Succeed())
		Expect(k8sClient.Create(ctx, &rbacv1.RoleBinding{
			ObjectMeta: metav1.ObjectMeta{Name: roleBindingGlobalRead, Namespace: globalA},
			RoleRef:    rbacv1.RoleRef{APIGroup: rbacv1.GroupName, Kind: kindClusterRole, Name: clusterRoleReadWrite},
			Subjects:   []rbacv1.Subject{{APIGroup: rbacv1.GroupName, Kind: rbacv1.GroupKind, Name: testGroupAuthenticated}},
		})).To(Succeed())

		reconcileOnce()

		read, err = getRB(globalA, roleBindingGlobalRead)
		Expect(err).NotTo(HaveOccurred())
		Expect(read.RoleRef.Name).To(Equal(clusterRoleRead))
		Expect(read.Labels).To(HaveKeyWithValue(labelGlobalNamespace, valueTrue))
	})

	It("follows Auth CR group changes", func() {
		setAuth([]string{testGroupAuthenticated, "team-x"}, []string{testGroupAdmins, testGroupPlatformAdmin})
		reconcileOnce()

		read, err := getRB(globalA, roleBindingGlobalRead)
		Expect(err).NotTo(HaveOccurred())
		Expect(subjectNames(read)).To(Equal([]string{testGroupPlatformAdmin, testGroupAdmins, testGroupAuthenticated, "team-x"}))
		write, err := getRB(globalA, roleBindingGlobalReadWrite)
		Expect(err).NotTo(HaveOccurred())
		Expect(subjectNames(write)).To(Equal([]string{testGroupPlatformAdmin, testGroupAdmins}))

		setAuth([]string{testGroupAuthenticated}, []string{testGroupAdmins})
		reconcileOnce()
		expectManagedRoleBindings(globalA)
	})

	It("clears the subjects of a binding applied with none", func() {
		empty := desiredGlobalRoleBindings(globalA, []string{testGroupAuthenticated}, nil, nil, nil)
		Expect(r.applyRoleBindings(ctx, empty)).To(Succeed())

		write, err := getRB(globalA, roleBindingGlobalReadWrite)
		Expect(err).NotTo(HaveOccurred())
		Expect(write.Subjects).To(BeEmpty())

		reconcileOnce()
		write, err = getRB(globalA, roleBindingGlobalReadWrite)
		Expect(err).NotTo(HaveOccurred())
		Expect(subjectNames(write)).To(Equal([]string{testGroupAdmins}))
	})

	It("grants access in every labeled namespace", func() {
		setLabel(globalB, true)
		reconcileOnce()

		expectManagedRoleBindings(globalA)
		expectManagedRoleBindings(globalB)
		Expect(tenantsFor(gn)).To(Equal([]string{serviceNS, globalA, globalB}))
	})

	It("removes the bindings from a namespace that is unlabeled", func() {
		setLabel(globalA, false)
		reconcileOnce()

		expectNoManagedRoleBindings(globalA)
		expectManagedRoleBindings(globalB)
		Expect(tenantsFor(gn)).To(Equal([]string{serviceNS, globalB}))

		_, err := getRB(globalA, curatorsRB)
		Expect(err).NotTo(HaveOccurred(), "administrator-created bindings survive")
	})

	It("removes group access but keeps Secret access when the Auth CR is missing", func() {
		deleteAuth()
		reconcileOnce()

		for _, name := range []string{roleBindingGlobalRead, roleBindingGlobalReadWrite} {
			rb, err := getRB(globalB, name)
			Expect(err).NotTo(HaveOccurred())
			Expect(rb.Subjects).To(BeEmpty(), "%s should grant no group access", name)
		}
		for _, name := range []string{roleBindingGlobalRestSecretAccess, roleBindingGlobalFlightSecretAccess} {
			rb, err := getRB(globalB, name)
			Expect(err).NotTo(HaveOccurred())
			Expect(rb.Subjects).NotTo(BeEmpty(), "%s should keep the service account", name)
		}

		setAuth([]string{testGroupAuthenticated}, []string{testGroupAdmins})
		reconcileOnce()
		expectManagedRoleBindings(globalB)
	})

	It("keeps only the DataConnectService namespace as tenant when no namespace is labeled", func() {
		setLabel(globalB, false)
		reconcileOnce()

		expectNoManagedRoleBindings(globalB)
		Expect(tenantsFor(gn)).To(Equal([]string{serviceNS}))
	})

	It("removes the bindings when no DataConnectService is left", func() {
		setLabel(globalA, true)
		reconcileOnce()
		expectManagedRoleBindings(globalA)

		cr := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(k8sClient.Delete(ctx, cr)).To(Succeed())
		reconcileOnce()

		expectNoManagedRoleBindings(globalA)
	})
})
