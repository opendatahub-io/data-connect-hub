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
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

var _ = Describe("Data module application field ownership", func() {
	It("preserves application overrides when platform fields are applied and cleared", func() {
		const namespaceField = "namespace"
		ctx := context.Background()
		service := &dchv1alpha1.DataConnectService{
			ObjectMeta: metav1.ObjectMeta{Name: "module-field-ownership", Namespace: "default"},
			Spec: dchv1alpha1.DataConnectServiceSpec{
				RestService: &dchv1alpha1.ServiceOverrides{Replicas: ptr.To(int32(3))},
				Trace:       &dchv1alpha1.Trace{Exporter: "http://collector.example:4317"},
			},
		}
		Expect(k8sClient.Create(ctx, service, client.FieldOwner("application-admin"))).To(Succeed())
		DeferCleanup(func() {
			Expect(k8sClient.Delete(ctx, service)).To(Succeed())
		})
		key := client.ObjectKeyFromObject(service)
		desired := &unstructured.Unstructured{Object: map[string]any{
			"apiVersion":    dchv1alpha1.SchemeGroupVersion.String(),
			"kind":          "DataConnectService",
			testMetadataKey: map[string]any{resourceNameField: service.Name, namespaceField: service.Namespace},
			testSpecKey: map[string]any{
				"gateway":              map[string]any{resourceNameField: "platform-gateway", namespaceField: "platform-ingress"},
				"tokenReviewAudiences": []any{"https://issuer.example"},
			},
		}}
		Expect(k8sClient.Apply(ctx, client.ApplyConfigurationFromUnstructured(desired), client.FieldOwner("feastoperator"), client.ForceOwnership)).To(Succeed())
		Expect(k8sClient.Get(ctx, key, service)).To(Succeed())
		Expect(service.Spec.Gateway.Name).To(Equal("platform-gateway"))
		Expect(service.Spec.TokenReviewAudiences).To(Equal([]string{"https://issuer.example"}))
		Expect(service.Spec.RestService.Replicas).To(Equal(ptr.To(int32(3))))
		Expect(service.Spec.Trace.Exporter).To(Equal("http://collector.example:4317"))

		// Use a clean apply object, just as the module does on each reconcile.
		cleared := &unstructured.Unstructured{Object: map[string]any{
			"apiVersion":    dchv1alpha1.SchemeGroupVersion.String(),
			"kind":          "DataConnectService",
			testMetadataKey: map[string]any{resourceNameField: service.Name, namespaceField: service.Namespace},
			testSpecKey:     map[string]any{},
		}}
		Expect(k8sClient.Apply(ctx, client.ApplyConfigurationFromUnstructured(cleared), client.FieldOwner("feastoperator"), client.ForceOwnership)).To(Succeed())
		Expect(k8sClient.Get(ctx, key, service)).To(Succeed())
		Expect(service.Spec.Gateway).To(BeNil())
		Expect(service.Spec.TokenReviewAudiences).To(BeEmpty())
		Expect(service.Spec.RestService.Replicas).To(Equal(ptr.To(int32(3))))
		Expect(service.Spec.Trace.Exporter).To(Equal("http://collector.example:4317"))
	})
})
