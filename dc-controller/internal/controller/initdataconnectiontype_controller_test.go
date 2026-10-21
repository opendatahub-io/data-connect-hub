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
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// mockConnectionTypeClient is a test double for ConnectionTypeClient.
type mockConnectionTypeClient struct {
	createFn    func(ctx context.Context, tenantID string, ct ConnectionType) error
	createCalls int
	lastCreate  ConnectionType

	listFn func(ctx context.Context, tenantID string) ([]ConnectionTypeResource, error)

	updateFn     func(ctx context.Context, tenantID string, id string, ct ConnectionType) error
	updateCalls  int
	lastUpdate   ConnectionType
	lastUpdateID string
}

func (m *mockConnectionTypeClient) CreateConnectionType(ctx context.Context, tenantID string, ct ConnectionType) error {
	m.createCalls++
	m.lastCreate = ct
	if m.createFn != nil {
		return m.createFn(ctx, tenantID, ct)
	}
	return nil
}

func (m *mockConnectionTypeClient) ListConnectionTypes(ctx context.Context, tenantID string) ([]ConnectionTypeResource, error) {
	if m.listFn != nil {
		return m.listFn(ctx, tenantID)
	}
	return nil, nil
}

func (m *mockConnectionTypeClient) UpdateConnectionType(ctx context.Context, tenantID string, id string, ct ConnectionType) error {
	m.updateCalls++
	m.lastUpdate = ct
	m.lastUpdateID = id
	if m.updateFn != nil {
		return m.updateFn(ctx, tenantID, id, ct)
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
		desc := testDescription
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

	It("should requeue when create conflicts so the next reconcile converges", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return ErrConflict
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnServiceUnavailable))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Pending"))
	})

	It("should patch the stored connection type when it differs from the spec", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		other := "Different description"
		stored := ConnectionTypeResource{
			Metadata: ResourceMetadata{ID: testResourceID, TenantID: resourceNamespace},
			Resource: ConnectionType{
				Name:        cr.Spec.Name,
				Provider:    cr.Spec.Provider,
				Description: &other,
				CredentialsFields: []Field{
					{Name: testFieldName, Label: testFieldLabel, Required: true, Type: testFieldType},
				},
			},
		}
		mock := &mockConnectionTypeClient{
			listFn: func(_ context.Context, _ string) ([]ConnectionTypeResource, error) {
				return []ConnectionTypeResource{stored}, nil
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.createCalls).To(Equal(0))
		Expect(mock.updateCalls).To(Equal(1))
		Expect(mock.lastUpdateID).To(Equal(testResourceID))
		Expect(mock.lastUpdate.Name).To(Equal(cr.Spec.Name))
		Expect(*mock.lastUpdate.Description).To(Equal(*cr.Spec.Description))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Completed"))
		Expect(cr.Status.Conditions[0].Reason).To(Equal("Updated"))
	})

	It("should not rewrite the stored connection type when it already matches", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		stored := ConnectionTypeResource{
			Metadata: ResourceMetadata{ID: testResourceID, TenantID: resourceNamespace},
			Resource: ConnectionType{
				Name:        cr.Spec.Name,
				Provider:    cr.Spec.Provider,
				Description: cr.Spec.Description,
				CredentialsFields: []Field{
					{Name: testFieldName, Label: testFieldLabel, Required: true, Type: testFieldType},
				},
			},
		}
		mock := &mockConnectionTypeClient{
			listFn: func(_ context.Context, _ string) ([]ConnectionTypeResource, error) {
				return []ConnectionTypeResource{stored}, nil
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.createCalls).To(Equal(0))
		Expect(mock.updateCalls).To(Equal(0))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Completed"))
		Expect(cr.Status.Conditions[0].Reason).To(Equal("UpToDate"))
	})

	It("should update the REST service when the CR spec changes", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		// First reconcile registers the type.
		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))
		Expect(mock.updateCalls).To(Equal(0))

		// Simulate the stored type still holding the previous spec.
		previous := testDescription
		mock.listFn = func(_ context.Context, _ string) ([]ConnectionTypeResource, error) {
			return []ConnectionTypeResource{{
				Metadata: ResourceMetadata{ID: testResourceID, TenantID: resourceNamespace},
				Resource: ConnectionType{
					Name:              cr.Spec.Name,
					Provider:          cr.Spec.Provider,
					Description:       &previous,
					CredentialsFields: []Field{},
				},
			}}, nil
		}

		// Update the CR spec — generation bumps, so the next reconcile syncs.
		next := "Updated by the manifest"
		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		cr.Spec.Description = &next
		Expect(k8sClient.Update(ctx, cr)).To(Succeed())

		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(mock.updateCalls).To(Equal(1))
		Expect(*mock.lastUpdate.Description).To(Equal(next))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Completed"))
	})

	It("should requeue when the stored type is deleted concurrently", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())

		mock := &mockConnectionTypeClient{
			listFn: func(_ context.Context, _ string) ([]ConnectionTypeResource, error) {
				return []ConnectionTypeResource{{
					Metadata: ResourceMetadata{ID: testResourceID, TenantID: resourceNamespace},
					Resource: ConnectionType{Name: cr.Spec.Name, Provider: cr.Spec.Provider, CredentialsFields: []Field{}},
				}}, nil
			},
			updateFn: func(_ context.Context, _ string, _ string, _ ConnectionType) error {
				return ErrNotFound
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnServiceUnavailable))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Pending"))
	})

	It("should re-sync a legacy AlreadyExists CR so it converges to the spec", func() {
		cr := newCR()
		Expect(k8sClient.Create(ctx, cr)).To(Succeed())
		// Simulate a CR recorded by an older controller version: the stored
		// type pre-existed and was never aligned with the spec.
		cr.Status.Phase = "AlreadyExists"
		meta.SetStatusCondition(&cr.Status.Conditions, metav1.Condition{
			Type:               conditionTypeSynced,
			Status:             metav1.ConditionTrue,
			Reason:             "AlreadyExists",
			Message:            "Connection type already exists in the REST service",
			ObservedGeneration: cr.Generation,
		})
		Expect(k8sClient.Status().Update(ctx, cr)).To(Succeed())

		other := "Different description"
		mock := &mockConnectionTypeClient{
			listFn: func(_ context.Context, _ string) ([]ConnectionTypeResource, error) {
				return []ConnectionTypeResource{{
					Metadata: ResourceMetadata{ID: testResourceID, TenantID: resourceNamespace},
					Resource: ConnectionType{
						Name:              cr.Spec.Name,
						Provider:          cr.Spec.Provider,
						Description:       &other,
						CredentialsFields: []Field{},
					},
				}}, nil
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: crKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.updateCalls).To(Equal(1))

		Expect(k8sClient.Get(ctx, crKey, cr)).To(Succeed())
		Expect(cr.Status.Phase).To(Equal("Completed"))
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
