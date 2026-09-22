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
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestHTTPClientVerifiesServiceCertificate(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusCreated)
	}))
	defer server.Close()

	roots := x509.NewCertPool()
	roots.AddCert(server.Certificate())
	client := newHTTPClientWithRootCAs(func() (string, error) { return server.URL, nil }, roots)
	if err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType()); err != nil {
		t.Fatalf("CreateConnectionType() returned an error: %v", err)
	}
}

func TestHTTPClientRejectsUnknownServiceCertificate(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusCreated)
	}))
	defer server.Close()

	client := newHTTPClientWithRootCAs(func() (string, error) { return server.URL, nil }, x509.NewCertPool())
	if err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType()); err != ErrServiceUnavailable {
		t.Fatalf("CreateConnectionType() error = %v, want ErrServiceUnavailable", err)
	}
}

func TestHTTPClientDoesNotDisableVerificationOrALPN(t *testing.T) {
	client := newHTTPClientWithRootCAs(func() (string, error) { return "http://localhost", nil }, x509.NewCertPool())
	transport := client.httpClient.Transport.(*http.Transport)
	if transport.TLSClientConfig.InsecureSkipVerify {
		t.Fatal("InsecureSkipVerify is enabled")
	}
	if transport.TLSClientConfig.NextProtos != nil {
		t.Fatalf("NextProtos = %v, want nil", transport.TLSClientConfig.NextProtos)
	}
	if !transport.ForceAttemptHTTP2 {
		t.Fatal("ForceAttemptHTTP2 = false, want true")
	}
}

func TestHTTPClientReloadsServiceCARotation(t *testing.T) {
	caPath := filepath.Join(t.TempDir(), "service-ca.crt")
	var serviceURL string
	client := newHTTPClientWithServiceCAPath(func() (string, error) { return serviceURL, nil }, caPath)

	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusCreated)
	}))
	serviceURL = server.URL
	writeServiceCA(t, caPath, server.Certificate())
	require.NoError(t, client.CreateConnectionType(context.Background(), "test-ns", testConnectionType()))
	server.Close()

	rotatedServer := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusCreated)
	}))
	defer rotatedServer.Close()
	serviceURL = rotatedServer.URL
	writeServiceCA(t, caPath, rotatedServer.Certificate())
	require.NoError(t, client.CreateConnectionType(context.Background(), "test-ns", testConnectionType()))
}

func writeServiceCA(t *testing.T, path string, certificate *x509.Certificate) {
	t.Helper()
	data := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificate.Raw})
	require.NoError(t, os.WriteFile(path, data, 0o600))
}

const (
	testProvider   = "test"
	testFieldName  = "HOST"
	testFieldLabel = "Host"
	testFieldType  = "string"
)

func testConnectionType() ConnectionType {
	desc := "Test connection type"
	return ConnectionType{
		Name:        "test-type",
		Provider:    testProvider,
		Description: &desc,
		CredentialsFields: []Field{
			{
				Name:     testFieldName,
				Label:    testFieldLabel,
				Required: true,
				Type:     testFieldType,
			},
		},
	}
}

func TestCreateConnectionType(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodPost, r.Method)
		assert.Equal(t, "/api/v1alpha1/data/connection-types", r.URL.Path)
		assert.Equal(t, "application/json", r.Header.Get("Content-Type"))
		assert.Equal(t, "test-ns", r.Header.Get("x-tenant-id"))

		var body ConnectionType
		require.NoError(t, json.NewDecoder(r.Body).Decode(&body))
		assert.Equal(t, "test-type", body.Name)

		w.WriteHeader(http.StatusCreated)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(func() (string, error) { return server.URL, nil })
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.NoError(t, err)
}

func TestCreateConnectionTypeConflict(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusConflict)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(func() (string, error) { return server.URL, nil })
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.ErrorIs(t, err, ErrConflict)
}

func TestServiceUnavailable(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(func() (string, error) { return server.URL, nil })
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.ErrorIs(t, err, ErrServiceUnavailable)
}

func TestConnectionRefused(t *testing.T) {
	client := NewHTTPConnectionTypeClient(func() (string, error) { return "http://localhost:1", nil })
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.ErrorIs(t, err, ErrServiceUnavailable)
}

func TestListConnectionTypes(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodGet, r.Method)
		assert.Equal(t, "/api/v1alpha1/data/connection-types", r.URL.Path)
		assert.Equal(t, "test-ns", r.Header.Get("x-tenant-id"))

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{
			"total_count": 1,
			"items": [{
				"metadata": {"id": "abc-123", "tenant_id": "test-ns", "created_at": "2026-01-01", "updated_at": "2026-01-01"},
				"resource": {"name": "s3", "provider": "s3", "credentials_fields": []}
			}]
		}`))
	}))
	defer server.Close()

	c := NewHTTPMigrationClient(func() (string, error) { return server.URL, nil })
	types, err := c.ListConnectionTypes(context.Background(), "test-ns")
	require.NoError(t, err)
	assert.Len(t, types, 1)
	assert.Equal(t, "abc-123", types[0].Metadata.ID)
	assert.Equal(t, "s3", types[0].Resource.Name)
}

func TestListConnectionTypesServiceUnavailable(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer server.Close()

	c := NewHTTPMigrationClient(func() (string, error) { return server.URL, nil })
	_, err := c.ListConnectionTypes(context.Background(), "test-ns")
	assert.ErrorIs(t, err, ErrServiceUnavailable)
}

func TestCreateConnection(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodPost, r.Method)
		assert.Equal(t, "/api/v1alpha1/data/connections", r.URL.Path)
		assert.Equal(t, "test-ns", r.Header.Get("x-tenant-id"))

		var body Connection
		require.NoError(t, json.NewDecoder(r.Body).Decode(&body))
		assert.Equal(t, "my-conn", body.Name)
		assert.Equal(t, "type-id", body.DataConnectionTypeID)
		assert.Equal(t, "my-secret", body.CredentialsRef.Secret)

		w.WriteHeader(http.StatusCreated)
	}))
	defer server.Close()

	c := NewHTTPMigrationClient(func() (string, error) { return server.URL, nil })
	err := c.CreateConnection(context.Background(), "test-ns", Connection{
		Name:                 "my-conn",
		DataConnectionTypeID: "type-id",
		Format:               "tabular",
		CredentialsRef:       &CredentialsRef{Secret: "my-secret"},
		Properties:           map[string]string{},
	})
	assert.NoError(t, err)
}

func TestCreateConnectionConflict(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusConflict)
	}))
	defer server.Close()

	c := NewHTTPMigrationClient(func() (string, error) { return server.URL, nil })
	err := c.CreateConnection(context.Background(), "test-ns", Connection{
		Name:       "my-conn",
		Properties: map[string]string{},
	})
	assert.ErrorIs(t, err, ErrConflict)
}
