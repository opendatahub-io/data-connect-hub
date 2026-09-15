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
	"strings"
	"testing"
	"time"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	"github.com/pelletier/go-toml/v2"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
)

const (
	testSQLiteConnector              = "sqlite"
	testTOMLEnabledKey               = "enabled"
	testTOMLConnectionTimeoutSecsKey = "connection_timeout_secs"
)

func flightServiceConfigMap(configTOML string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		"kind": "ConfigMap",
		"metadata": map[string]any{
			"labels": map[string]any{
				"app.kubernetes.io/name": "flight-service",
			},
		},
		"data": map[string]any{
			"config.toml": configTOML,
		},
	}}
}

func TestSetConfigMapFlightConnectorSettingsAddConnector(t *testing.T) {
	// Add connector settings for connectors that are not in the base configuration.
	disabled := false
	enabled := true
	tests := []struct {
		name          string
		connectorName string
		enabled       *bool
		configTOML    string
		expected      string
	}{
		{
			name:          "disabled SQLite",
			connectorName: testSQLiteConnector,
			enabled:       &disabled,
			configTOML: `
[connectors.default]
enabled = false
`,
			expected: "[connectors.sqlite]\nenabled = false",
		},
		{
			name:          "enabled custom connector",
			connectorName: "custom",
			enabled:       &enabled,
			configTOML: `
[connectors.default]
enabled = false
`,
			expected: "[connectors.custom]\nenabled = true",
		},
		{
			name:          "missing enabled",
			connectorName: testSQLiteConnector,
			configTOML: `
[connectors.default]
enabled = true
`,
			expected: "[connectors.sqlite]\nenabled = false",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			configMap := flightServiceConfigMap(tt.configTOML)

			if err := setConfigMapFlightConnectorSettings([]*unstructured.Unstructured{configMap}, &dchv1alpha1.ServiceOverrides{
				Connectors: []dchv1alpha1.ConnectorConfig{{Name: tt.connectorName, Enabled: tt.enabled}},
			}); err != nil {
				t.Fatal(err)
			}

			data, found, err := unstructured.NestedStringMap(configMap.Object, "data")
			if err != nil || !found {
				t.Fatalf("expected ConfigMap data, found=%v err=%v", found, err)
			}
			if !strings.Contains(data["config.toml"], tt.expected) {
				t.Fatalf("expected %s, got:\n%s", tt.expected, data["config.toml"])
			}
		})
	}
}

func TestSetConfigMapFlightConnectorSettingsUpdateConnector(t *testing.T) {
	// Update specified connector settings while preserving unspecified connector settings.
	enabled := true
	timeout := &metav1.Duration{Duration: 30 * time.Second}
	configTOML := `
[connectors.default]
enabled = false
connection_timeout_secs = 10

[connectors.postgres]
enabled = true
connection_timeout_secs = 30

[connectors.s3]
enabled = true
chunk_size = 1024

[connectors.sqlite]
enabled = false
connection_timeout_secs = 10

[connectors.neo4j]
enabled = false
connection_timeout_secs = 20
`
	configMap := flightServiceConfigMap(configTOML)

	if err := setConfigMapFlightConnectorSettings([]*unstructured.Unstructured{configMap}, &dchv1alpha1.ServiceOverrides{
		Connectors: []dchv1alpha1.ConnectorConfig{
			{Name: testSQLiteConnector, Enabled: &enabled, ConnectionTimeout: timeout},
			{Name: "neo4j", Enabled: &enabled},
		},
	}); err != nil {
		t.Fatal(err)
	}

	data, found, err := unstructured.NestedStringMap(configMap.Object, "data")
	if err != nil || !found {
		t.Fatalf("expected ConfigMap data, found=%v err=%v", found, err)
	}
	var updated map[string]any
	if err := toml.Unmarshal([]byte(data["config.toml"]), &updated); err != nil {
		t.Fatalf("expected valid updated TOML: %v", err)
	}
	connectors, ok := updated["connectors"].(map[string]any)
	if !ok {
		t.Fatalf("expected connectors table, got %#v", updated["connectors"])
	}
	assertConnector := func(name string, expected map[string]any) {
		t.Helper()
		connector, ok := connectors[name].(map[string]any)
		if !ok {
			t.Fatalf("expected %s connector table, got %#v", name, connectors[name])
		}
		for key, expectedValue := range expected {
			if connector[key] != expectedValue {
				t.Errorf("expected connectors.%s.%s=%v, got %v", name, key, expectedValue, connector[key])
			}
		}
	}
	assertConnector("postgres", map[string]any{
		testTOMLEnabledKey:               true,
		testTOMLConnectionTimeoutSecsKey: int64(30),
	})
	assertConnector("s3", map[string]any{
		testTOMLEnabledKey: true,
		"chunk_size":       int64(1024),
	})
	assertConnector(testSQLiteConnector, map[string]any{
		testTOMLEnabledKey:               true,
		testTOMLConnectionTimeoutSecsKey: int64(30),
	})
	assertConnector("neo4j", map[string]any{
		testTOMLEnabledKey:               true,
		testTOMLConnectionTimeoutSecsKey: int64(20),
	})
	if connectors["default"].(map[string]any)["enabled"] != false {
		t.Errorf("expected connectors.default.enabled=false, got %v", connectors["default"].(map[string]any)["enabled"])
	}
}
