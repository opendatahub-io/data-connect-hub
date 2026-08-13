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
)

const (
	maxResponseBodyBytes = 1 << 20 // 1 MiB
	saTokenPath          = "/var/run/secrets/kubernetes.io/serviceaccount/token"
)

var (
	ErrConflict           = errors.New("resource already exists")
	ErrServiceUnavailable = errors.New("rest service unavailable")
)

// ConnectionTypeClient abstracts REST calls to the connection-type endpoints.
type ConnectionTypeClient interface {
	CreateConnectionType(ctx context.Context, tenantID string, ct ConnectionType) error
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

type httpConnectionTypeClient struct {
	baseURL   string
	tokenPath string

	httpClient *http.Client
}

// NewHTTPConnectionTypeClient creates a ConnectionTypeClient that calls the
// rest-service through kube-rbac-proxy over HTTPS. It reads the service
// account token on each request (Kubernetes rotates projected tokens).
func NewHTTPConnectionTypeClient(baseURL string) ConnectionTypeClient {
	return &httpConnectionTypeClient{
		baseURL:   baseURL,
		tokenPath: saTokenPath,
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

func (c *httpConnectionTypeClient) CreateConnectionType(ctx context.Context, tenantID string, ct ConnectionType) error {
	body, err := json.Marshal(ct)
	if err != nil {
		return fmt.Errorf("marshaling connection type: %w", err)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.baseURL+"/api/v1/data/connection-types", bytes.NewReader(body))
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
