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

func TestCreateConnectionType(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		assert.Equal(t, http.MethodPost, r.Method)
		assert.Equal(t, "/api/v1/data/connection-types", r.URL.Path)
		assert.Equal(t, "application/json", r.Header.Get("Content-Type"))
		assert.Equal(t, "test-ns", r.Header.Get("x-tenant-id"))

		var body ConnectionType
		require.NoError(t, json.NewDecoder(r.Body).Decode(&body))
		assert.Equal(t, "test-type", body.Name)

		w.WriteHeader(http.StatusCreated)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL)
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.NoError(t, err)
}

func TestCreateConnectionTypeConflict(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusConflict)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL)
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.ErrorIs(t, err, ErrConflict)
}

func TestServiceUnavailable(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
	}))
	defer server.Close()

	client := NewHTTPConnectionTypeClient(server.URL)
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.ErrorIs(t, err, ErrServiceUnavailable)
}

func TestConnectionRefused(t *testing.T) {
	client := NewHTTPConnectionTypeClient("http://localhost:1")
	err := client.CreateConnectionType(context.Background(), "test-ns", testConnectionType())
	assert.ErrorIs(t, err, ErrServiceUnavailable)
}
