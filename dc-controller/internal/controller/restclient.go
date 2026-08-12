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
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"time"
)

var (
	ErrNotFound           = errors.New("resource not found")
	ErrServiceUnavailable = errors.New("rest service unavailable")
)

// ConnectionTypeClient abstracts REST calls to the connection-type endpoints.
type ConnectionTypeClient interface {
	CreateConnectionType(ctx context.Context, ct ConnectionType) (*ConnectionTypeResource, error)
	GetConnectionType(ctx context.Context, id string) (*ConnectionTypeResource, error)
	UpdateConnectionType(ctx context.Context, id string, ct ConnectionType) (*ConnectionTypeResource, error)
	DeleteConnectionType(ctx context.Context, id string) error
}

// ConnectionType mirrors the Rust DataConnectionType JSON structure.
type ConnectionType struct {
	Name              string  `json:"name"`
	Provider          string  `json:"provider"`
	Description       *string `json:"description,omitempty"`
	CredentialsFields []Field `json:"credentials_fields"`
}

// Field mirrors the Rust Field JSON structure.
type Field struct {
	Name         string      `json:"name"`
	Label        string      `json:"label"`
	Description  *string     `json:"description,omitempty"`
	Required     bool        `json:"required"`
	Type         string      `json:"type"`
	EnumValues   []EnumValue `json:"enum_values,omitempty"`
	DefaultValue *string     `json:"default_value,omitempty"`
}

// EnumValue mirrors the Rust EnumValue JSON structure.
type EnumValue struct {
	Value string `json:"value"`
	Label string `json:"label"`
}

// ResourceMetadata mirrors the Rust ResourceMetadata JSON structure.
type ResourceMetadata struct {
	ID        string `json:"id"`
	TenantID  string `json:"tenant_id"`
	CreatedAt string `json:"created_at"`
	UpdatedAt string `json:"updated_at"`
}

// ConnectionTypeResource is the REST API response for a connection type.
type ConnectionTypeResource struct {
	Metadata ResourceMetadata `json:"metadata"`
	Resource ConnectionType   `json:"resource"`
}

type httpConnectionTypeClient struct {
	baseURL    string
	httpClient *http.Client
	tenantID   string
}

// NewHTTPConnectionTypeClient creates a ConnectionTypeClient that calls the REST service over HTTP.
func NewHTTPConnectionTypeClient(baseURL, tenantID string) ConnectionTypeClient {
	return &httpConnectionTypeClient{
		baseURL:  baseURL,
		tenantID: tenantID,
		httpClient: &http.Client{
			Timeout: 10 * time.Second,
		},
	}
}

func (c *httpConnectionTypeClient) CreateConnectionType(ctx context.Context, ct ConnectionType) (*ConnectionTypeResource, error) {
	body, err := json.Marshal(ct)
	if err != nil {
		return nil, fmt.Errorf("marshaling connection type: %w", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.baseURL+"/api/v1/data/connection-types", bytes.NewReader(body))
	if err != nil {
		return nil, fmt.Errorf("creating request: %w", err)
	}
	c.setHeaders(req)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, ErrServiceUnavailable
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusCreated {
		var resource ConnectionTypeResource
		if err := json.NewDecoder(resp.Body).Decode(&resource); err != nil {
			return nil, fmt.Errorf("decoding response: %w", err)
		}
		return &resource, nil
	}

	return nil, c.handleErrorResponse(resp)
}

func (c *httpConnectionTypeClient) GetConnectionType(ctx context.Context, id string) (*ConnectionTypeResource, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.baseURL+"/api/v1/data/connection-types/"+id, nil)
	if err != nil {
		return nil, fmt.Errorf("creating request: %w", err)
	}
	c.setHeaders(req)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, ErrServiceUnavailable
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusOK {
		var resource ConnectionTypeResource
		if err := json.NewDecoder(resp.Body).Decode(&resource); err != nil {
			return nil, fmt.Errorf("decoding response: %w", err)
		}
		return &resource, nil
	}

	return nil, c.handleErrorResponse(resp)
}

func (c *httpConnectionTypeClient) UpdateConnectionType(ctx context.Context, id string, ct ConnectionType) (*ConnectionTypeResource, error) {
	body, err := json.Marshal(ct)
	if err != nil {
		return nil, fmt.Errorf("marshaling connection type: %w", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPatch, c.baseURL+"/api/v1/data/connection-types/"+id, bytes.NewReader(body))
	if err != nil {
		return nil, fmt.Errorf("creating request: %w", err)
	}
	c.setHeaders(req)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, ErrServiceUnavailable
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusOK {
		var resource ConnectionTypeResource
		if err := json.NewDecoder(resp.Body).Decode(&resource); err != nil {
			return nil, fmt.Errorf("decoding response: %w", err)
		}
		return &resource, nil
	}

	return nil, c.handleErrorResponse(resp)
}

func (c *httpConnectionTypeClient) DeleteConnectionType(ctx context.Context, id string) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodDelete, c.baseURL+"/api/v1/data/connection-types/"+id, nil)
	if err != nil {
		return fmt.Errorf("creating request: %w", err)
	}
	c.setHeaders(req)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return ErrServiceUnavailable
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusNoContent {
		return nil
	}

	return c.handleErrorResponse(resp)
}

func (c *httpConnectionTypeClient) setHeaders(req *http.Request) {
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("x-tenant-id", c.tenantID)
}

func (c *httpConnectionTypeClient) handleErrorResponse(resp *http.Response) error {
	if resp.StatusCode == http.StatusNotFound {
		return ErrNotFound
	}
	if resp.StatusCode >= 500 {
		return ErrServiceUnavailable
	}
	body, _ := io.ReadAll(resp.Body)
	return fmt.Errorf("unexpected status %d: %s", resp.StatusCode, string(body))
}
