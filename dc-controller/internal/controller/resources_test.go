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

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
)

const (
	testKindKey        = "kind"
	testMetadataKey    = "metadata"
	testFlightCACMName = "flight-service-ca"
	testDataKey        = "data"
	testConfigTOMLKey  = "config.toml"
	testSpecKey        = "spec"
	testNamespace      = "test-ns"
)

func TestAnnotateDeploymentsWithContentHash(t *testing.T) {
	s := runtime.NewScheme()
	_ = corev1.AddToScheme(s)

	tlsSecret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "default-dcs-flight-tls", Namespace: testNamespace},
		Data:       map[string][]byte{"tls.crt": []byte("cert"), "tls.key": []byte("key")},
	}
	dbSecret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: nameDatabaseConfig, Namespace: testNamespace},
		Data:       map[string][]byte{nameSecretConfigTOML: []byte("url=postgres://...")},
	}

	fakeClient := fake.NewClientBuilder().WithScheme(s).WithObjects(tlsSecret, dbSecret).Build()
	r := &DataConnectServiceReconciler{Client: fakeClient}

	configMap := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			keyName: "default-dcs-flight-config",
		},
		testDataKey: map[string]any{
			testConfigTOMLKey: "[connectors.uri]\nenabled = false\n",
		},
	}}
	deployment := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindDeployment,
		testMetadataKey: map[string]any{
			keyName: testFlightServiceName,
		},
		testSpecKey: map[string]any{
			"template": map[string]any{
				testSpecKey: map[string]any{
					"containers": []any{
						map[string]any{
							keyName: "flight-server",
							"image": "localhost/dch-flight:test",
						},
					},
					"volumes": []any{
						map[string]any{
							keyName:     "config",
							"configMap": map[string]any{keyName: "default-dcs-flight-config"},
						},
						map[string]any{
							keyName:  "tls",
							"secret": map[string]any{keySecretName: "default-dcs-flight-tls"},
						},
						map[string]any{
							keyName:  "db-secret",
							"secret": map[string]any{keySecretName: nameDatabaseConfig},
						},
						map[string]any{
							keyName:    "tmp",
							"emptyDir": map[string]any{},
						},
					},
				},
			},
		},
	}}

	resources := []*unstructured.Unstructured{configMap, deployment}
	if err := r.annotateDeploymentsWithContentHash(context.Background(), resources, testNamespace); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	annotations, found, err := unstructured.NestedStringMap(
		deployment.Object, testSpecKey, "template", testMetadataKey, "annotations",
	)
	if err != nil || !found {
		t.Fatalf("expected deployment template annotations, found=%v err=%v", found, err)
	}
	hash1 := annotations[annotationConfigHash]
	if hash1 == "" {
		t.Fatal("expected dataconnecthub/config-hash annotation on deployment")
	}

	tlsSecret2 := tlsSecret.DeepCopy()
	tlsSecret2.Data["tls.crt"] = []byte("rotated-cert")
	fakeClient2 := fake.NewClientBuilder().WithScheme(s).WithObjects(tlsSecret2, dbSecret).Build()
	r2 := &DataConnectServiceReconciler{Client: fakeClient2}

	deployment2 := deployment.DeepCopy()
	resources2 := []*unstructured.Unstructured{configMap, deployment2}
	if err := r2.annotateDeploymentsWithContentHash(context.Background(), resources2, testNamespace); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	annotations2, _, _ := unstructured.NestedStringMap(
		deployment2.Object, testSpecKey, "template", testMetadataKey, "annotations",
	)
	if annotations2[annotationConfigHash] == hash1 {
		t.Fatal("expected hash to change when secret data changes")
	}
}

func TestAnnotateDeploymentsWithContentHash_FallsBackToLiveConfigMap(t *testing.T) {
	s := runtime.NewScheme()
	_ = corev1.AddToScheme(s)

	liveCA := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Name: testFlightCACMName, Namespace: testNamespace},
		Data:       map[string]string{"service-ca.crt": "-----BEGIN CERTIFICATE-----\nMIIC...\n-----END CERTIFICATE-----\n"},
	}
	fakeClient := fake.NewClientBuilder().WithScheme(s).WithObjects(liveCA).Build()
	r := &DataConnectServiceReconciler{Client: fakeClient}

	emptyCA := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			keyName: testFlightCACMName,
		},
		testDataKey: map[string]any{},
	}}
	deployment := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindDeployment,
		testMetadataKey: map[string]any{
			keyName: "rest-service",
		},
		testSpecKey: map[string]any{
			"template": map[string]any{
				testSpecKey: map[string]any{
					"containers": []any{
						map[string]any{keyName: "rest-server", "image": "test:latest"},
					},
					"volumes": []any{
						map[string]any{
							keyName:     "flight-ca",
							"configMap": map[string]any{keyName: testFlightCACMName},
						},
					},
				},
			},
		},
	}}

	resources := []*unstructured.Unstructured{emptyCA, deployment}
	if err := r.annotateDeploymentsWithContentHash(context.Background(), resources, testNamespace); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	annotations, found, _ := unstructured.NestedStringMap(
		deployment.Object, testSpecKey, "template", testMetadataKey, "annotations",
	)
	if !found || annotations[annotationConfigHash] == "" {
		t.Fatal("expected hash annotation from live-fetched CA ConfigMap")
	}
}
