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
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// mockConnectionTypeClient is a test double for ConnectionTypeClient.
type mockConnectionTypeClient struct {
	createFn func(ctx context.Context, ct ConnectionType) (*ConnectionTypeResource, error)
	getFn    func(ctx context.Context, id string) (*ConnectionTypeResource, error)
	updateFn func(ctx context.Context, id string, ct ConnectionType) (*ConnectionTypeResource, error)
	deleteFn func(ctx context.Context, id string) error

	createCalls int
	getCalls    int
	updateCalls int
	deleteCalls int
}

func (m *mockConnectionTypeClient) CreateConnectionType(ctx context.Context, ct ConnectionType) (*ConnectionTypeResource, error) {
	m.createCalls++
	if m.createFn != nil {
		return m.createFn(ctx, ct)
	}
	return &ConnectionTypeResource{
		Metadata: ResourceMetadata{ID: "generated-uuid"},
		Resource: ct,
	}, nil
}

func (m *mockConnectionTypeClient) GetConnectionType(ctx context.Context, id string) (*ConnectionTypeResource, error) {
	m.getCalls++
	if m.getFn != nil {
		return m.getFn(ctx, id)
	}
	return nil, ErrNotFound
}

func (m *mockConnectionTypeClient) UpdateConnectionType(ctx context.Context, id string, ct ConnectionType) (*ConnectionTypeResource, error) {
	m.updateCalls++
	if m.updateFn != nil {
		return m.updateFn(ctx, id, ct)
	}
	return &ConnectionTypeResource{
		Metadata: ResourceMetadata{ID: id},
		Resource: ct,
	}, nil
}

func (m *mockConnectionTypeClient) DeleteConnectionType(ctx context.Context, id string) error {
	m.deleteCalls++
	if m.deleteFn != nil {
		return m.deleteFn(ctx, id)
	}
	return nil
}

var _ = Describe("InitDataConnectionType Controller", func() {
	const resourceName = "test-idct"
	ctx := context.Background()
	crKey := types.NamespacedName{Name: resourceName}

	newCR := func() *dchv1alpha1.InitDataConnectionType {
		desc := "Test connection type"
		return &dchv1alpha1.InitDataConnectionType{
			ObjectMeta: metav1.ObjectMeta{
				Name: resourceName,
			},
			Spec: dchv1alpha1.InitDataConnectionTypeSpec{
				Name:        "TestType",
				Provider:    "test",
				Description: &desc,
				CredentialsFields: []dchv1alpha1.CredentialsField{
					{
						Name:     "HOST",
						Label:    "Host",
						Required: true,
						Type:     "string",
					},
				},
			},
		}
	}

	cleanup := func() {
		cr := &dchv1alpha1.InitDataConnectionType{}
		if err := k8sClient.Get(ctx, crKey, cr); err == nil {
			controllerutil.RemoveFinalizer(cr, idctFinalizerName)
			_ = k8sClient.Update(ctx, cr)
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

	It("should add finalizer, create connection type, and set status in a single reconcile", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		// Single reconcile does everything: finalizer + create + status
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.createCalls).To(Equal(1))

		// Verify CR state
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(controllerutil.ContainsFinalizer(cr, idctFinalizerName)).To(BeTrue())
		Expect(cr.Annotations[annotationResourceID]).To(Equal("generated-uuid"))
		Expect(cr.Status.Phase).To(Equal("Synced"))
		Expect(cr.Status.Conditions).To(HaveLen(1))
		Expect(cr.Status.Conditions[0].Type).To(Equal(conditionTypeSynced))
		Expect(cr.Status.Conditions[0].Status).To(Equal(metav1.ConditionTrue))
	})

	It("should be idempotent when resource exists and matches", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		// First reconcile creates the type
		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		// Now set up mock to return the existing resource on GET
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		desired := specToConnectionType(&cr.Spec)
		mock.getFn = func(_ context.Context, id string) (*ConnectionTypeResource, error) {
			return &ConnectionTypeResource{
				Metadata: ResourceMetadata{ID: id},
				Resource: desired,
			}, nil
		}

		// Second reconcile should be a no-op
		mock.createCalls = 0
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.getCalls).To(Equal(1))
		Expect(mock.createCalls).To(Equal(0))
		Expect(mock.updateCalls).To(Equal(0))
	})

	It("should call update when spec has changed", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		// First reconcile creates
		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		// Mock returns stale resource (different description)
		oldDesc := "Old description"
		mock.getFn = func(_ context.Context, id string) (*ConnectionTypeResource, error) {
			return &ConnectionTypeResource{
				Metadata: ResourceMetadata{ID: id},
				Resource: ConnectionType{
					Name:        "TestType",
					Provider:    "test",
					Description: &oldDesc,
					CredentialsFields: []Field{
						{Name: "HOST", Label: "Host", Required: true, Type: "string"},
					},
				},
			}, nil
		}

		// Second reconcile detects delta and updates
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.getCalls).To(Equal(1))
		Expect(mock.updateCalls).To(Equal(1))
	})

	It("should re-create when resource was deleted externally", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		// First reconcile creates
		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		// Mock now returns 404 on GET (deleted externally)
		mock.getFn = func(_ context.Context, _ string) (*ConnectionTypeResource, error) {
			return nil, ErrNotFound
		}
		mock.createFn = func(_ context.Context, ct ConnectionType) (*ConnectionTypeResource, error) {
			return &ConnectionTypeResource{
				Metadata: ResourceMetadata{ID: "new-uuid"},
				Resource: ct,
			}, nil
		}
		mock.createCalls = 0

		// Second reconcile re-creates
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.createCalls).To(Equal(1))

		// Annotation should be updated with new UUID
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Annotations[annotationResourceID]).To(Equal("new-uuid"))
	})

	It("should requeue when REST service is unavailable", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ ConnectionType) (*ConnectionTypeResource, error) {
				return nil, ErrServiceUnavailable
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnServiceUnavailable))

		// Status should reflect pending state
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Pending"))
	})

	It("should delete REST resource and remove finalizer on CR deletion", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		// Reconcile creates
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		// Delete the CR
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(k8sClient.Delete(ctx, cr)).To(Succeed())

		// Reconcile handles deletion
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.deleteCalls).To(Equal(1))
	})

	It("should remove finalizer even when REST resource is already gone", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			deleteFn: func(_ context.Context, _ string) error {
				return ErrNotFound
			},
		}
		r := reconciler(mock)

		// Reconcile adds finalizer + creates (create uses default mock, returns success)
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		// Delete the CR
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(k8sClient.Delete(ctx, cr)).To(Succeed())

		// Reconcile should succeed — 404 from REST is acceptable
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.deleteCalls).To(Equal(1))
	})

	It("should requeue on deletion when REST service is unavailable", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		// Reconcile creates
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		// Now make delete fail
		mock.deleteFn = func(_ context.Context, _ string) error {
			return ErrServiceUnavailable
		}

		// Delete the CR
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(k8sClient.Delete(ctx, cr)).To(Succeed())

		// Reconcile should requeue — can't remove finalizer until REST confirms
		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnServiceUnavailable))

		// Finalizer should still be present
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(controllerutil.ContainsFinalizer(cr, idctFinalizerName)).To(BeTrue())
	})

	It("should set error status on sync failure", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ ConnectionType) (*ConnectionTypeResource, error) {
				return nil, fmt.Errorf("unexpected error from REST")
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnError))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Error"))
	})
})
