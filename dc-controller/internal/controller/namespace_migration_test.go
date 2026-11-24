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
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"

	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

type namespaceMigrationTransport struct {
	backend      *url.URL
	expectedHost string
	base         http.RoundTripper
}

func (t namespaceMigrationTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	if req.URL.Host != t.expectedHost {
		return nil, fmt.Errorf("migration used REST host %q instead of %q", req.URL.Host, t.expectedHost)
	}
	copy := req.Clone(req.Context())
	endpoint := *req.URL
	endpoint.Scheme, endpoint.Host = t.backend.Scheme, t.backend.Host
	copy.URL = &endpoint
	return t.base.RoundTrip(copy)
}

// A legacy Secret predates DCH and stays in its tenant namespace. Only the DCH
// metadata is created through REST discovered in the new infrastructure namespace.
func TestLegacyConnectionMigrationWithDedicatedInfrastructureNamespace(t *testing.T) {
	for _, infrastructure := range []string{"odh-data-connect-hub-infra", "rhoai-data-connect-hub-infra"} {
		t.Run(infrastructure, func(t *testing.T) {
			const tenant = "legacy-team"
			const legacySecretName = "legacy-uri-connection"
			const legacyTypeID = "legacy-uri-type-id"
			scheme := runtime.NewScheme()
			require.NoError(t, corev1.AddToScheme(scheme))
			require.NoError(t, dchv1alpha1.AddToScheme(scheme))
			secret := &corev1.Secret{ObjectMeta: metav1.ObjectMeta{
				Name: legacySecretName, Namespace: tenant,
				Labels:      map[string]string{labelODHDashboard: valueSyncedTrue},
				Annotations: map[string]string{annotationConnectionTypeRef: cmNameURIv1, annotationDisplayName: "Legacy URI"},
			}, Data: map[string][]byte{"URI": []byte("https://example.invalid/legacy"), "TOKEN": []byte("preserved-secret-value")}}
			originalData := secret.DeepCopy().Data
			legacyType := &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{
				Name: cmNameURIv1, Namespace: "redhat-ods-applications",
				Labels: map[string]string{labelODHConnectionType: valueSyncedTrue},
			}}
			kube := fake.NewClientBuilder().WithScheme(scheme).WithObjects(secret, legacyType).Build()
			created := make(chan Connection, 1)
			var requests atomic.Int32
			server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				requests.Add(1)
				if req.Header.Get("x-tenant-id") != tenant || req.Header.Get("Authorization") != "Bearer migration-test-token" {
					http.Error(w, "incorrect migration tenant or authentication", http.StatusForbidden)
					return
				}
				w.Header().Set("Content-Type", "application/json")
				switch {
				case req.Method == http.MethodGet && req.URL.Path == "/api/v1alpha1/data/connection-types":
					_, _ = fmt.Fprintf(w, `{"total_count":1,"items":[{"metadata":{"id":%q},"resource":{"name":"URI","provider":"uri"}}]}`, legacyTypeID)
				case req.Method == http.MethodPost && req.URL.Path == "/api/v1alpha1/data/connections":
					var connection Connection
					if err := json.NewDecoder(req.Body).Decode(&connection); err != nil {
						http.Error(w, err.Error(), http.StatusBadRequest)
						return
					}
					created <- connection
					w.WriteHeader(http.StatusCreated)
				default:
					http.NotFound(w, req)
				}
			}))
			defer server.Close()
			backend, err := url.Parse(server.URL)
			require.NoError(t, err)
			tokenPath := filepath.Join(t.TempDir(), "token")
			require.NoError(t, os.WriteFile(tokenPath, []byte("migration-test-token"), 0o600))
			httpClient := server.Client()
			httpClient.Transport = namespaceMigrationTransport{
				backend: backend, expectedHost: "dch-rest-service." + infrastructure + ".svc.cluster.local:8443", base: httpClient.Transport,
			}
			restClient := &httpConnectionTypeClient{resolveURL: NewRestServiceURLResolver(kube), tokenPath: tokenPath, httpClient: httpClient}
			watcher := &SecretWatcherReconciler{Client: kube, Scheme: scheme, RestClient: restClient}
			request := ctrl.Request{NamespacedName: types.NamespacedName{Namespace: tenant, Name: legacySecretName}}
			result, err := watcher.Reconcile(t.Context(), request)
			require.NoError(t, err)
			require.Positive(t, result.RequeueAfter, "legacy connections must wait for DCH, not be marked migrated")
			require.Zero(t, requests.Load())
			application := &dchv1alpha1.DataConnectService{ObjectMeta: metav1.ObjectMeta{Name: "default-dataconnectservice", Namespace: infrastructure}}
			require.NoError(t, kube.Create(t.Context(), application))
			typeWatcher := &ConfigMapWatcherReconciler{Client: kube, Scheme: scheme, RestClient: restClient}
			_, err = typeWatcher.Reconcile(t.Context(), ctrl.Request{NamespacedName: client.ObjectKeyFromObject(legacyType)})
			require.NoError(t, err)
			_, err = watcher.Reconcile(t.Context(), request)
			require.NoError(t, err)
			require.Equal(t, int32(2), requests.Load())
			connection := <-created
			require.Equal(t, "Legacy URI", connection.Name)
			require.Equal(t, legacyTypeID, connection.DataConnectionTypeID)
			require.NotNil(t, connection.CredentialsRef)
			require.Equal(t, legacySecretName, connection.CredentialsRef.Secret)
			require.NoError(t, kube.Get(t.Context(), request.NamespacedName, secret))
			require.Equal(t, originalData, secret.Data, "migration must not rewrite credentials")
			require.Equal(t, valueSyncedTrue, secret.Annotations[annotationDCHSynced])
			var secrets corev1.SecretList
			require.NoError(t, kube.List(t.Context(), &secrets))
			require.Len(t, secrets.Items, 1, "credentials must not be copied into the infrastructure namespace")
			_, err = watcher.Reconcile(t.Context(), request)
			require.NoError(t, err)
			require.Equal(t, int32(2), requests.Load(), "already migrated connections must not be duplicated")
		})
	}
}
