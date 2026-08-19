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

var _ = Describe("ConfigMap Watcher Controller", func() {
	const (
		cmName      = "test-odh-conntype"
		cmNamespace = "default"
	)
	ctx := context.Background()
	cmKey := types.NamespacedName{Name: cmName, Namespace: cmNamespace}

	newConfigMap := func() *corev1.ConfigMap {
		return &corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{
				Name:      cmName,
				Namespace: cmNamespace,
				Labels: map[string]string{
					labelODHConnectionType: valueSyncedTrue,
				},
				Annotations: map[string]string{
					annotationDisplayName: "Test S3 Storage",
					annotationDescription: "S3-compatible object storage",
				},
			},
			Data: map[string]string{
				"category": `["Object storage"]`,
				"fields": `[
					{"type":"short-text","name":"Access key","envVar":"AWS_ACCESS_KEY_ID","required":true,"properties":{}},
					{"type":"password","name":"Secret key","envVar":"AWS_SECRET_ACCESS_KEY","required":true,"properties":{}}
				]`,
			},
		}
	}

	cleanup := func() {
		cm := &corev1.ConfigMap{}
		if err := k8sClient.Get(ctx, cmKey, cm); err == nil {
			_ = k8sClient.Delete(ctx, cm)
		}
	}

	AfterEach(func() {
		cleanup()
	})

	reconciler := func(mock *mockConnectionTypeClient) *ConfigMapWatcherReconciler {
		return &ConfigMapWatcherReconciler{
			Client:     k8sClient,
			Scheme:     k8sClient.Scheme(),
			RestClient: mock,
		}
	}

	It("should promote ConfigMap to connection type and mark synced", func() {
		cm := newConfigMap()
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		var captured ConnectionType
		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, tenantID string, ct ConnectionType) error {
				Expect(tenantID).To(Equal(cmNamespace))
				captured = ct
				return nil
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		Expect(captured.Name).To(Equal(cmName))
		Expect(captured.Provider).To(Equal(cmName))
		Expect(captured.Description).To(HaveValue(Equal("S3-compatible object storage")))
		Expect(captured.CredentialsFields).To(HaveLen(2))
		Expect(captured.CredentialsFields[0].Name).To(Equal("AWS_ACCESS_KEY_ID"))
		Expect(captured.CredentialsFields[0].Label).To(Equal("Access key"))
		Expect(captured.CredentialsFields[0].Type).To(Equal("string"))
		Expect(captured.CredentialsFields[0].Required).To(BeTrue())

		Expect(k8sClient.Get(ctx, cmKey, cm)).To(Succeed())
		Expect(cm.Annotations[annotationDCHSynced]).To(Equal(valueSyncedTrue))
	})

	It("should verify already-synced ConfigMap still exists in REST", func() {
		cm := newConfigMap()
		cm.Annotations[annotationDCHSynced] = valueSyncedTrue
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return ErrConflict
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))
	})

	It("should mark synced on conflict (already exists)", func() {
		cm := newConfigMap()
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return ErrConflict
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, cmKey, cm)).To(Succeed())
		Expect(cm.Annotations[annotationDCHSynced]).To(Equal(valueSyncedTrue))
	})

	It("should requeue when REST service is unavailable", func() {
		cm := newConfigMap()
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return ErrServiceUnavailable
			},
		}
		r := reconciler(mock)

		result, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(result.RequeueAfter).To(Equal(requeueOnMigrationServiceUnavailable))

		Expect(k8sClient.Get(ctx, cmKey, cm)).To(Succeed())
		Expect(cm.Annotations).NotTo(HaveKey(annotationDCHSynced))
	})

	It("should skip ConfigMap without data.fields", func() {
		cm := newConfigMap()
		delete(cm.Data, "fields")
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(0))
	})

	It("should skip ConfigMap with invalid fields JSON", func() {
		cm := newConfigMap()
		cm.Data["fields"] = "not-json"
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(0))
	})

	It("should re-create after database reset when synced annotation exists", func() {
		cm := newConfigMap()
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		mock := &mockConnectionTypeClient{}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(1))

		// Second reconcile still attempts create (verifies type exists)
		_, err = r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())
		Expect(mock.createCalls).To(Equal(2))
	})

	It("should not set error on unexpected REST failure", func() {
		cm := newConfigMap()
		Expect(k8sClient.Create(ctx, cm)).To(Succeed())

		mock := &mockConnectionTypeClient{
			createFn: func(_ context.Context, _ string, _ ConnectionType) error {
				return fmt.Errorf("unexpected error")
			},
		}
		r := reconciler(mock)

		_, err := r.Reconcile(ctx, reconcile.Request{NamespacedName: cmKey})
		Expect(err).NotTo(HaveOccurred())

		Expect(k8sClient.Get(ctx, cmKey, cm)).To(Succeed())
		Expect(cm.Annotations).NotTo(HaveKey(annotationDCHSynced))
	})
})
