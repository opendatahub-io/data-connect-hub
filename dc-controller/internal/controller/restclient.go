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
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"

	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	maxResponseBodyBytes = 1 << 20 // 1 MiB
	saTokenPath          = "/var/run/secrets/kubernetes.io/serviceaccount/token"
)

var (
	ErrConflict           = errors.New("resource already exists")
	ErrServiceUnavailable = errors.New("rest service unavailable")
	ErrNotFound           = errors.New("resource not found")
)

// ConnectionTypeClient abstracts REST calls to the connection-type endpoints.
type ConnectionTypeClient interface {
	CreateConnectionType(ctx context.Context, tenantID string, ct ConnectionType) error
}

// ConnectionMigrationClient abstracts REST calls needed by the Secret migration watcher.
type ConnectionMigrationClient interface {
	ListConnectionTypes(ctx context.Context, tenantID string) ([]ConnectionTypeResource, error)
	CreateConnection(ctx context.Context, tenantID string, conn Connection) error
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

// Connection mirrors the Rust DataConnection JSON structure.
type Connection struct {
	Name                 string            `json:"name"`
	DataConnectionTypeID string            `json:"data_connection_type_id"`
	Format               string            `json:"format"`
	CredentialsRef       *CredentialsRef   `json:"credentials_ref"`
	Properties           map[string]string `json:"properties"`
}

// CredentialsRef holds a reference to a Kubernetes Secret.
type CredentialsRef struct {
	Secret string `json:"secret"`
}

// ResourceMetadata mirrors the Rust ResourceMetadata JSON structure.
type ResourceMetadata struct {
	ID        string `json:"id"`
	TenantID  string `json:"tenant_id"`
	CreatedAt string `json:"created_at"`
	UpdatedAt string `json:"updated_at"`
}

// ConnectionTypeResource mirrors the Rust DataConnectionTypeResource JSON structure.
type ConnectionTypeResource struct {
	Metadata ResourceMetadata `json:"metadata"`
	Resource ConnectionType   `json:"resource"`
}

// connectionTypeListResponse is the envelope for GET /connection-types.
type connectionTypeListResponse struct {
	TotalCount int                      `json:"total_count"`
	Items      []ConnectionTypeResource `json:"items"`
}

// URLResolver returns the base URL for the REST service. It is called on
// each request so the URL can be derived dynamically (e.g. from the
// DataConnectService CR's namespace).
type URLResolver func() (string, error)

type httpConnectionTypeClient struct {
	resolveURL URLResolver
	tokenPath  string

	httpClient *http.Client
}

func newHTTPClient(resolver URLResolver) *httpConnectionTypeClient {
	return &httpConnectionTypeClient{
		resolveURL: resolver,
		tokenPath:  saTokenPath,
		httpClient: &http.Client{
			Timeout: 10 * time.Second,
			Transport: &http.Transport{
				TLSClientConfig: &tls.Config{
					InsecureSkipVerify: true, //nolint:gosec // in-cluster service communication
					NextProtos:         []string{"http/1.1"},
				},
			},
		},
	}
}

// NewHTTPConnectionTypeClient creates a ConnectionTypeClient that calls the
// rest-service through kube-rbac-proxy over HTTPS. The URL is resolved
// dynamically via the provided resolver.
func NewHTTPConnectionTypeClient(resolver URLResolver) ConnectionTypeClient {
	return newHTTPClient(resolver)
}

// NewHTTPMigrationClient creates a ConnectionMigrationClient for the
// Secret watcher to list connection types and create connections.
func NewHTTPMigrationClient(resolver URLResolver) ConnectionMigrationClient {
	return newHTTPClient(resolver)
}

func (c *httpConnectionTypeClient) baseURL() (string, error) {
	return c.resolveURL()
}

func (c *httpConnectionTypeClient) CreateConnectionType(ctx context.Context, tenantID string, ct ConnectionType) error {
	url, err := c.baseURL()
	if err != nil {
		return ErrServiceUnavailable
	}

	body, err := json.Marshal(ct)
	if err != nil {
		return fmt.Errorf("marshaling connection type: %w", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, url+"/api/v1alpha1/data/connection-types", bytes.NewReader(body))
	if err != nil {
		return fmt.Errorf("creating request: %w", err)
	}
	c.setHeaders(req, tenantID)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return ErrServiceUnavailable
	}
	defer resp.Body.Close() //nolint:errcheck

	if resp.StatusCode == http.StatusCreated {
		return nil
	}
	if resp.StatusCode == http.StatusConflict {
		return ErrConflict
	}
	if resp.StatusCode >= 500 {
		return ErrServiceUnavailable
	}

	respBody, _ := io.ReadAll(io.LimitReader(resp.Body, maxResponseBodyBytes))
	return fmt.Errorf("unexpected status %d: %s", resp.StatusCode, string(respBody))
}

func (c *httpConnectionTypeClient) ListConnectionTypes(ctx context.Context, tenantID string) ([]ConnectionTypeResource, error) {
	url, err := c.baseURL()
	if err != nil {
		return nil, ErrServiceUnavailable
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url+"/api/v1alpha1/data/connection-types", nil)
	if err != nil {
		return nil, fmt.Errorf("creating request: %w", err)
	}
	c.setHeaders(req, tenantID)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, ErrServiceUnavailable
	}
	defer resp.Body.Close() //nolint:errcheck

	body, _ := io.ReadAll(io.LimitReader(resp.Body, maxResponseBodyBytes))

	if resp.StatusCode >= 500 {
		return nil, ErrServiceUnavailable
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("unexpected status %d: %s", resp.StatusCode, string(body))
	}

	var listResp connectionTypeListResponse
	if err := json.Unmarshal(body, &listResp); err != nil {
		return nil, fmt.Errorf("decoding connection types: %w", err)
	}
	return listResp.Items, nil
}

func (c *httpConnectionTypeClient) CreateConnection(ctx context.Context, tenantID string, conn Connection) error {
	url, err := c.baseURL()
	if err != nil {
		return ErrServiceUnavailable
	}

	body, err := json.Marshal(conn)
	if err != nil {
		return fmt.Errorf("marshaling connection: %w", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, url+"/api/v1alpha1/data/connections", bytes.NewReader(body))
	if err != nil {
		return fmt.Errorf("creating request: %w", err)
	}
	c.setHeaders(req, tenantID)

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return ErrServiceUnavailable
	}
	defer resp.Body.Close() //nolint:errcheck

	if resp.StatusCode == http.StatusCreated {
		return nil
	}
	if resp.StatusCode == http.StatusConflict {
		return ErrConflict
	}
	if resp.StatusCode >= 500 {
		return ErrServiceUnavailable
	}

	respBody, _ := io.ReadAll(io.LimitReader(resp.Body, maxResponseBodyBytes))
	return fmt.Errorf("unexpected status %d: %s", resp.StatusCode, string(respBody))
}

// NewRestServiceURLResolver returns a URLResolver that discovers the REST service
// URL by listing DataConnectService CRs and constructing the in-cluster service URL
// from the singleton CR's namespace.
func NewRestServiceURLResolver(k8sClient client.Client) URLResolver {
	return func() (string, error) {
		var list dchv1alpha1.DataConnectServiceList
		if err := k8sClient.List(context.Background(), &list); err != nil {
			return "", fmt.Errorf("listing DataConnectService CRs: %w", err)
		}
		if len(list.Items) == 0 {
			return "", fmt.Errorf("no DataConnectService CR found")
		}
		ns := list.Items[0].Namespace
		return fmt.Sprintf("https://dch-rest-service.%s.svc.cluster.local:8443", ns), nil
	}
}

func (c *httpConnectionTypeClient) setHeaders(req *http.Request, tenantID string) {
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("x-tenant-id", tenantID)

	if token, err := c.readToken(); err == nil && token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
}

func (c *httpConnectionTypeClient) readToken() (string, error) {
	data, err := os.ReadFile(c.tokenPath)
	if err != nil {
		if os.IsNotExist(err) {
			return "", nil
		}
		return "", err
	}
	return strings.TrimSpace(string(data)), nil
}
