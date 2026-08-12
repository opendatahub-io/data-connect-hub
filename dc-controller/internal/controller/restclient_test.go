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
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

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

func testConnectionTypeResource() ConnectionTypeResource {
	return ConnectionTypeResource{
		Metadata: ResourceMetadata{
			ID:        "uuid-123",
			TenantID:  "",
			CreatedAt: "2026-01-01T00:00:00Z",
			UpdatedAt: "2026-01-01T00:00:00Z",
		},
		Resource: testConnectionType(),
	}
}

func TestCreateConnectionType(t *testing.T) {
	resource := testConnectionTypeResource()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodPost, r.Method)
		assert.Equal(t, "/api/v1/data/connection-types", r.URL.Path)
		assert.Equal(t, "application/json", r.Header.Get("Content-Type"))
		assert.Equal(t, "", r.Header.Get("x-tenant-id"))

		var body ConnectionType
		require.NoError(t, json.NewDecoder(r.Body).Decode(&body))
		assert.Equal(t, "test-type", body.Name)

		w.WriteHeader(http.StatusCreated)
		_ = json.NewEncoder(w).Encode(resource)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL, "")
	result, err := client.CreateConnectionType(context.Background(), testConnectionType())
	require.NoError(t, err)
	assert.Equal(t, "uuid-123", result.Metadata.ID)
	assert.Equal(t, "test-type", result.Resource.Name)
}

func TestGetConnectionType(t *testing.T) {
	resource := testConnectionTypeResource()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodGet, r.Method)
		assert.Equal(t, "/api/v1/data/connection-types/uuid-123", r.URL.Path)

		w.WriteHeader(http.StatusOK)
		_ = json.NewEncoder(w).Encode(resource)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL, "")
	result, err := client.GetConnectionType(context.Background(), "uuid-123")
	require.NoError(t, err)
	assert.Equal(t, "uuid-123", result.Metadata.ID)
}

func TestGetConnectionTypeNotFound(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL, "")
	_, err := client.GetConnectionType(context.Background(), "missing-id")
	assert.ErrorIs(t, err, ErrNotFound)
}

func TestUpdateConnectionType(t *testing.T) {
	resource := testConnectionTypeResource()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodPut, r.Method)
		assert.Equal(t, "/api/v1/data/connection-types/uuid-123", r.URL.Path)

		var body ConnectionType
		require.NoError(t, json.NewDecoder(r.Body).Decode(&body))
		assert.Equal(t, "test-type", body.Name)

		w.WriteHeader(http.StatusOK)
		_ = json.NewEncoder(w).Encode(resource)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL, "")
	result, err := client.UpdateConnectionType(context.Background(), "uuid-123", testConnectionType())
	require.NoError(t, err)
	assert.Equal(t, "uuid-123", result.Metadata.ID)
}

func TestDeleteConnectionType(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodDelete, r.Method)
		assert.Equal(t, "/api/v1/data/connection-types/uuid-123", r.URL.Path)

		w.WriteHeader(http.StatusNoContent)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL, "")
	err := client.DeleteConnectionType(context.Background(), "uuid-123")
	assert.NoError(t, err)
}

func TestDeleteConnectionTypeNotFound(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL, "")
	err := client.DeleteConnectionType(context.Background(), "missing-id")
	assert.ErrorIs(t, err, ErrNotFound)
}

func TestServiceUnavailable(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL, "")

	_, err := client.CreateConnectionType(context.Background(), testConnectionType())
	assert.ErrorIs(t, err, ErrServiceUnavailable)

	_, err = client.GetConnectionType(context.Background(), "id")
	assert.ErrorIs(t, err, ErrServiceUnavailable)

	err = client.DeleteConnectionType(context.Background(), "id")
	assert.ErrorIs(t, err, ErrServiceUnavailable)
}

func TestConnectionRefused(t *testing.T) {
	client := NewHTTPConnectionTypeClient("http://localhost:1", "")

	_, err := client.CreateConnectionType(context.Background(), testConnectionType())
	assert.ErrorIs(t, err, ErrServiceUnavailable)
}
