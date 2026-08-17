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
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"
)

type mockMigrationClient struct {
	listFn      func(ctx context.Context, tenantID string) ([]ConnectionTypeResource, error)
	createFn    func(ctx context.Context, tenantID string, conn Connection) error
	listCalls   int
	createCalls int
}

func (m *mockMigrationClient) ListConnectionTypes(ctx context.Context, tenantID string) ([]ConnectionTypeResource, error) {
	m.listCalls++
	if m.listFn != nil {
		return m.listFn(ctx, tenantID)
	}
	return []ConnectionTypeResource{
		{
			Metadata: ResourceMetadata{ID: "type-uuid-123"},
			Resource: ConnectionType{Name: "s3", Provider: "s3"},
		},
	}, nil
}

func (m *mockMigrationClient) CreateConnection(ctx context.Context, tenantID string, conn Connection) error {
	m.createCalls++
	if m.createFn != nil {
		return m.createFn(ctx, tenantID, conn)
	}
	return nil
}

var _ = Describe("Secret Watcher Controller", func() {
	const (
		secretName      = "test-odh-secret"
		secretNamespace = "default"
	)
	ctx := context.Background()
	secretKey := types.NamespacedName{Name: secretName, Namespace: secretNamespace}

	newSecret := func() *corev1.Secret {
		return &corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{
				Name:      secretName,
				Namespace: secretNamespace,
				Labels: map[string]string{
					labelODHDashboard: valueSyncedTrue,
				},
				Annotations: map[string]string{
					annotationDisplayName:       "My S3 Connection",
					annotationConnectionTypeRef: "s3",
				},
			},
			Data: map[string][]byte{
				"AWS_ACCESS_KEY_ID":     []byte("test-key"),
				"AWS_SECRET_ACCESS_KEY": []byte("test-secret"),
			},
			Type: corev1.SecretTypeOpaque,
		}
	}

	cleanup := func() {
		secret := &corev1.Secret{}
		if err := k8sClient.Get(ctx, secretKey, secret); err == nil {
			_ = k8sClient.Delete(ctx, secret)
		}
	}

	AfterEach(func() {
		cleanup()
	})

	reconciler := func(mock *mockMigrationClient) *SecretWatcherReconciler {
		return &SecretWatcherReconciler{
			Client:     k8sClient,
			Scheme:     k8sClient.Scheme(),
			RestClient: mock,
		}
	}

	It("should migrate Secret and mark synced", func() {
		secret := newSecret()
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		var captured Connection
		mock := &mockMigrationClient{
			createFn: func(_ context.Context, tenantID string, conn Connection) error {
				Expect(tenantID).To(Equal(secretNamespace))
				captured = conn
				return nil
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.listCalls).To(Equal(1))
		Expect(mock.createCalls).To(Equal(1))

		Expect(captured.Name).To(Equal("My S3 Connection"))
		Expect(captured.DataConnectionTypeID).To(Equal("type-uuid-123"))
		Expect(captured.Format).To(Equal("tabular"))
		Expect(captured.Admin).NotTo(BeNil())
		Expect(captured.Admin.SecretRef).To(Equal(secretName))

		Expect(k8sClient.Get(ctx, secretKey, secret)).To(Succeed())
		Expect(secret.Annotations[annotationDCHSynced]).To(Equal(valueSyncedTrue))
	})

	It("should use Secret name when display-name annotation is absent", func() {
		secret := newSecret()
		delete(secret.Annotations, annotationDisplayName)
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		var captured Connection
		mock := &mockMigrationClient{
			createFn: func(_ context.Context, _ string, conn Connection) error {
				captured = conn
				return nil
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(captured.Name).To(Equal(secretName))
	})

	It("should skip already-synced Secret", func() {
		secret := newSecret()
		secret.Annotations[annotationDCHSynced] = valueSyncedTrue
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.listCalls).To(Equal(0))
		Expect(mock.createCalls).To(Equal(0))
	})

	It("should skip Secret without connection-type-ref", func() {
		secret := newSecret()
		delete(secret.Annotations, annotationConnectionTypeRef)
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.listCalls).To(Equal(0))
	})

	It("should requeue when connection type is not yet migrated", func() {
		secret := newSecret()
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{
			listFn: func(_ context.Context, _ string) ([]ConnectionTypeResource, error) {
				return []ConnectionTypeResource{}, nil
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnMigrationServiceUnavailable))
	})

	It("should requeue when REST service is unavailable on list", func() {
		secret := newSecret()
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{
			listFn: func(_ context.Context, _ string) ([]ConnectionTypeResource, error) {
				return nil, ErrServiceUnavailable
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnMigrationServiceUnavailable))
	})

	It("should requeue when REST service is unavailable on create", func() {
		secret := newSecret()
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{
			createFn: func(_ context.Context, _ string, _ Connection) error {
				return ErrServiceUnavailable
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnMigrationServiceUnavailable))
	})

	It("should mark synced on conflict (already exists)", func() {
		secret := newSecret()
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{
			createFn: func(_ context.Context, _ string, _ Connection) error {
				return ErrConflict
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, secretKey, secret)).To(Succeed())
		Expect(secret.Annotations[annotationDCHSynced]).To(Equal(valueSyncedTrue))
	})

	It("should not re-create after successful sync", func() {
		secret := newSecret()
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))
	})

	It("should not set synced on unexpected REST failure", func() {
		secret := newSecret()
		Expect(k8sClient.Create(ctx, secret)).To(Succeed())

		mock := &mockMigrationClient{
			createFn: func(_ context.Context, _ string, _ Connection) error {
				return fmt.Errorf("unexpected error")
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: secretKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, secretKey, secret)).To(Succeed())
		Expect(secret.Annotations).NotTo(HaveKey(annotationDCHSynced))
	})
})
