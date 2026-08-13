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
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// mockConnectionTypeClient is a test double for ConnectionTypeClient.
type mockConnectionTypeClient struct {
	createFn    func(ctx context.Context, tenantID string, ct ConnectionType) error
	createCalls int
}

func (m *mockConnectionTypeClient) CreateConnectionType(ctx context.Context, tenantID string, ct ConnectionType) error {
	m.createCalls++
	if m.createFn != nil {
		return m.createFn(ctx, tenantID, ct)
	}
	return nil
}

var _ = Describe("InitDataConnectionType Controller", func() {
	const (
		resourceName      = "test-idct"
		resourceNamespace = "default"
	)
	ctx := context.Background()
	crKey := types.NamespacedName{Name: resourceName, Namespace: resourceNamespace}

	newCR := func() *dchv1alpha1.InitDataConnectionType {
		desc := "Test connection type"
		return &dchv1alpha1.InitDataConnectionType{
			ObjectMeta: metav1.ObjectMeta{
				Name:      resourceName,
				Namespace: resourceNamespace,
			},
			Spec: dchv1alpha1.InitDataConnectionTypeSpec{
				Name:        "TestType",
				Provider:    testProvider,
				Description: &desc,
				CredentialsFields: []dchv1alpha1.CredentialsField{
					{
						Name:     testFieldName,
						Label:    testFieldLabel,
						Required: true,
						Type:     testFieldType,
					},
				},
			},
		}
	}

	cleanup := func() {
		cr := &dchv1alpha1.InitDataConnectionType{}
		if err := k8sClient.Get(ctx, crKey, cr); err == nil {
			_ = k8sClient.Delete(ctx, cr)
		}
	}

	AfterEach(func() {
		cleanup()
	})

	reconciler := func(mock *mockConnectionTypeClient) *InitDataConnectionTypeReconciler {
		return &InitDataConnectionTypeReconciler{
			Client:     k8sClient,
			Scheme:     k8sClient.Scheme(),
			RestClient: mock,
		}
	}

	It("should create connection type and set status to Completed", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Completed"))
		Expect(cr.Status.Conditions).To(HaveLen(1))
		Expect(cr.Status.Conditions[0].Type).To(Equal(conditionTypeSynced))
		Expect(cr.Status.Conditions[0].Status).To(Equal(metav1.ConditionTrue))
		Expect(cr.Status.Conditions[0].Reason).To(Equal("Created"))
	})

	It("should not re-create on subsequent reconcile after Completed", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		// First reconcile creates
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		// Second reconcile is a no-op (terminal state)
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))
	})

	It("should set AlreadyExists when REST returns 409", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return ErrConflict
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("AlreadyExists"))
	})

	It("should not retry after AlreadyExists", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return ErrConflict
			},
		}
		r := reconciler(mock)

		// First reconcile hits conflict
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		// Second reconcile is a no-op (terminal state)
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))
	})

	It("should requeue when REST service is unavailable", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return ErrServiceUnavailable
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnServiceUnavailable))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Pending"))
	})

	It("should set error status on unexpected failure", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return fmt.Errorf("unexpected error from REST")
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Error"))
	})

	It("should do nothing on CR deletion (no finalizer)", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		// Reconcile creates
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		// Delete the CR — no finalizer, so it's just gone
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(k8sClient.Delete(ctx, cr)).To(Succeed())

		// Reconcile sees NotFound — returns cleanly
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		// Only 1 create call total (no delete call)
		Expect(mock.createCalls).To(Equal(1))
	})
})
