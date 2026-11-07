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
	"k8s.io/apimachinery/pkg/api/meta"
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
		_, err := r.Reconcile(ctx, globalNamespaceReconcileKey)
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
		auth.SetGroupVersionKind(authGVK)
		err := k8sClient.Get(ctx, types.NamespacedName{Name: authCRName}, auth)
		if apierrors.IsNotFound(err) {
			auth.SetName(authCRName)
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
		auth.SetGroupVersionKind(authGVK)
		auth.SetName(authCRName)
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

	status := func() (string, *metav1.Condition) {
		cr := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		return cr.Status.GlobalNamespace, meta.FindStatusCondition(cr.Status.Conditions, conditionTypeGlobalNSReady)
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
			APIReader:    k8sClient,
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

	It("reports NotConfigured when no namespace is labeled", func() {
		reconcileOnce()
		expectNoManagedRoleBindings(globalA)
		ns, cond := status()
		Expect(ns).To(BeEmpty())
		Expect(cond).NotTo(BeNil())
		Expect(cond.Status).To(Equal(metav1.ConditionTrue))
		Expect(cond.Reason).To(Equal(reasonGlobalNSNotConfigured))
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

		restRB, err := getRB(globalA, roleBindingGlobalRestSecret)
		Expect(err).NotTo(HaveOccurred())
		Expect(restRB.Subjects).To(ContainElement(rbacv1.Subject{
			Kind: rbacv1.ServiceAccountKind, Namespace: serviceNS, Name: serviceAccountRestService,
		}))
		flightRB, err := getRB(globalA, roleBindingGlobalFlightSecret)
		Expect(err).NotTo(HaveOccurred())
		Expect(flightRB.Subjects).To(ContainElement(rbacv1.Subject{
			Kind: rbacv1.ServiceAccountKind, Namespace: serviceNS, Name: flightServiceAccountName(serviceCR),
		}))

		ns, cond := status()
		Expect(ns).To(Equal(globalA))
		Expect(cond.Status).To(Equal(metav1.ConditionTrue))
		Expect(cond.Reason).To(Equal(reasonGlobalNSReconciled))
	})

	It("uses the global namespace as the global connection type tenant", func() {
		cr := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		tenant, err := globalTenantFor(ctx, k8sClient, gn, cr)
		Expect(err).NotTo(HaveOccurred())
		Expect(tenant).To(Equal(globalA))

		tenant, err = connectionTypeTenantFor(ctx, k8sClient, gn, serviceNS)
		Expect(err).NotTo(HaveOccurred())
		Expect(tenant).To(Equal(globalA), "sources in the DataConnectService namespace follow the global tenant")

		tenant, err = connectionTypeTenantFor(ctx, k8sClient, gn, globalB)
		Expect(err).NotTo(HaveOccurred())
		Expect(tenant).To(Equal(globalB), "other namespaces are their own tenant")

		tenant, err = connectionTypeTenantFor(ctx, k8sClient, GlobalNamespaceConfig{LabelKey: labelKey}, serviceNS)
		Expect(err).NotTo(HaveOccurred())
		Expect(tenant).To(Equal(serviceNS), "disabled feature keeps today's behavior")
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

		By("dropping the read-write binding when adminGroups empties")
		setAuth([]string{testGroupAuthenticated}, nil)
		reconcileOnce()
		_, err = getRB(globalA, roleBindingGlobalReadWrite)
		Expect(apierrors.IsNotFound(err)).To(BeTrue())

		setAuth([]string{testGroupAuthenticated}, []string{testGroupAdmins})
		reconcileOnce()
		expectManagedRoleBindings(globalA)
	})

	It("keeps the current namespace when several are labeled", func() {
		setLabel(globalB, true)
		reconcileOnce()

		expectManagedRoleBindings(globalA)
		expectNoManagedRoleBindings(globalB)
		ns, cond := status()
		Expect(ns).To(Equal(globalA))
		Expect(cond.Status).To(Equal(metav1.ConditionFalse))
		Expect(cond.Reason).To(Equal(reasonGlobalNSMultiple))

		cr := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		tenant, err := globalTenantFor(ctx, k8sClient, gn, cr)
		Expect(err).NotTo(HaveOccurred())
		Expect(tenant).To(Equal(globalA))
	})

	It("moves the bindings when the designation moves", func() {
		setLabel(globalA, false)
		reconcileOnce()

		expectNoManagedRoleBindings(globalA)
		expectManagedRoleBindings(globalB)
		ns, cond := status()
		Expect(ns).To(Equal(globalB))
		Expect(cond.Reason).To(Equal(reasonGlobalNSReconciled))

		_, err := getRB(globalA, curatorsRB)
		Expect(err).NotTo(HaveOccurred(), "administrator-created bindings survive the move")
	})

	It("removes all access when the Auth CR is missing", func() {
		deleteAuth()
		reconcileOnce()

		expectNoManagedRoleBindings(globalB)
		_, cond := status()
		Expect(cond.Status).To(Equal(metav1.ConditionFalse))
		Expect(cond.Reason).To(Equal(reasonGlobalNSAuthNotFound))

		setAuth([]string{testGroupAuthenticated}, []string{testGroupAdmins})
		reconcileOnce()
		expectManagedRoleBindings(globalB)
	})

	It("removes the bindings and restores the default tenant when the designation is cleared", func() {
		setLabel(globalB, false)
		reconcileOnce()

		expectNoManagedRoleBindings(globalB)
		ns, cond := status()
		Expect(ns).To(BeEmpty())
		Expect(cond.Reason).To(Equal(reasonGlobalNSNotConfigured))

		cr := &dchv1alpha1.DataConnectService{}
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		tenant, err := globalTenantFor(ctx, k8sClient, gn, cr)
		Expect(err).NotTo(HaveOccurred())
		Expect(tenant).To(Equal(serviceNS))
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
