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
	"bufio"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	"github.com/pelletier/go-toml/v2"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"sigs.k8s.io/kustomize/kyaml/filesys"
)

const (
	testSQLiteConnector              = "sqlite"
	testTOMLEnabledKey               = "enabled"
	testTOMLConnectionTimeoutSecsKey = "connection_timeout_secs"
	testTOMLRequestTimeoutSecsKey    = "request_timeout_secs"
	testTOMLReadTimeoutSecsKey       = "read_timeout_secs"
	testKindKey                      = "kind"
	testMetadataKey                  = "metadata"
	testNameKey                      = "name"
	testRestImageParam               = "REST_IMAGE"
	testFlightImageParam             = "FLIGHT_IMAGE"
)

func flightServiceConfigMap(configTOML string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
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

			if err := setConfigMapFlightConnectorSettings([]*unstructured.Unstructured{configMap}, nameFlightService, &dchv1alpha1.ServiceOverrides{
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
	connectionTimeout := &metav1.Duration{Duration: 30 * time.Second}
	requestTimeout := &metav1.Duration{Duration: 45 * time.Second}
	readTimeout := &metav1.Duration{Duration: 15 * time.Second}
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

	if err := setConfigMapFlightConnectorSettings([]*unstructured.Unstructured{configMap}, nameFlightService, &dchv1alpha1.ServiceOverrides{
		Connectors: []dchv1alpha1.ConnectorConfig{
			{
				Name:              testSQLiteConnector,
				Enabled:           &enabled,
				ConnectionTimeout: connectionTimeout,
				RequestTimeout:    requestTimeout,
				ReadTimeout:       readTimeout,
			},
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
		testTOMLRequestTimeoutSecsKey:    int64(45),
		testTOMLReadTimeoutSecsKey:       int64(15),
	})
	assertConnector("neo4j", map[string]any{
		testTOMLEnabledKey:               true,
		testTOMLConnectionTimeoutSecsKey: int64(20),
	})
	if connectors["default"].(map[string]any)["enabled"] != false {
		t.Errorf("expected connectors.default.enabled=false, got %v", connectors["default"].(map[string]any)["enabled"])
	}
}

// TestRenderKustomizationManifestRoots renders each root the controller may
// build, through the in-memory staging that production uses, so that a
// kustomization reaching outside its own directory is caught here.
func TestRenderKustomizationManifestRoots(t *testing.T) {
	manifestsPath := filepath.Join("..", "..", "..", "config")

	tests := []struct {
		name               string
		path               string
		wantServiceMonitor bool
	}{
		{name: "base", path: filepath.Join(manifestsPath, "base")},
		{name: "openshift overlay", path: filepath.Join(manifestsPath, "overlays", "openshift"), wantServiceMonitor: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			resources, err := renderKustomization(manifestsPath, tt.path, nil, nil, nil)
			if err != nil {
				t.Fatalf("rendering %s: %v", tt.path, err)
			}

			var deployments, serviceMonitors int
			for _, obj := range resources {
				switch obj.GetKind() {
				case kindDeployment:
					deployments++
				case "ServiceMonitor":
					serviceMonitors++
				}
			}

			if deployments != 2 {
				t.Errorf("rendered Deployments = %d, want 2", deployments)
			}
			want := 0
			if tt.wantServiceMonitor {
				want = 1
			}
			if serviceMonitors != want {
				t.Errorf("rendered ServiceMonitors = %d, want %d", serviceMonitors, want)
			}
		})
	}
}

func TestRenderKustomizationImageParams(t *testing.T) {
	manifestsPath := filepath.Join("..", "..", "..", "config")
	paramsPath := filepath.Join(manifestsPath, "base", "params.env")
	originalParams, err := os.ReadFile(paramsPath)
	if err != nil {
		t.Fatal(err)
	}

	imageParams := map[string]string{
		RelatedImageRestService:   "registry.example.com/dch/rest@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		RelatedImageFlightService: "registry.example.com/dch/flight@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
		RelatedImageKubeRbacProxy: "registry.example.com/dch/kube-rbac-proxy@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
	}
	paths := []struct {
		name string
		path string
	}{
		{name: "base", path: filepath.Join(manifestsPath, "base")},
		{name: "openshift overlay", path: filepath.Join(manifestsPath, "overlays", "openshift")},
	}

	for _, tt := range paths {
		t.Run(tt.name, func(t *testing.T) {
			resources, err := renderKustomization(manifestsPath, tt.path, nil, nil, imageParams)
			if err != nil {
				t.Fatalf("rendering %s: %v", tt.path, err)
			}

			wantImages := map[string]string{
				nameRestService:   imageParams[RelatedImageRestService],
				nameFlightService: imageParams[RelatedImageFlightService],
				nameKubeRbacProxy: imageParams[RelatedImageKubeRbacProxy],
			}
			for containerName, wantImage := range wantImages {
				gotImage, found := renderedContainerImage(resources, containerName)
				if !found {
					t.Errorf("container %q not found in rendered Deployments", containerName)
					continue
				}
				if gotImage != wantImage {
					t.Errorf("container %q image = %q, want %q", containerName, gotImage, wantImage)
				}
			}
		})
	}

	updatedParams, err := os.ReadFile(paramsPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(updatedParams) != string(originalParams) {
		t.Fatal("rendering changed the source params.env on disk")
	}
}

func renderedContainerImage(resources []*unstructured.Unstructured, containerName string) (string, bool) {
	for _, obj := range resources {
		if obj.GetKind() != kindDeployment {
			continue
		}
		containers, found, err := unstructured.NestedSlice(obj.Object, "spec", "template", "spec", "containers")
		if err != nil || !found {
			continue
		}
		for _, raw := range containers {
			container, ok := raw.(map[string]any)
			if !ok || container["name"] != containerName {
				continue
			}
			image, _ := container["image"].(string)
			return image, true
		}
	}
	return "", false
}

func TestMergeParamsEnvPreservesExistingParams(t *testing.T) {
	fs := filesys.MakeFsInMemory()
	path := filepath.Join("/manifests", "params.env")
	if err := fs.MkdirAll(filepath.Dir(path)); err != nil {
		t.Fatal(err)
	}
	original := "# defaults\nREST_IMAGE=old:tag\nKEEP=preserved\nFLIGHT_IMAGE=old-flight@sha256:old\n"
	if err := fs.WriteFile(path, []byte(original)); err != nil {
		t.Fatal(err)
	}

	overrides := map[string]string{
		testRestImageParam:   "registry.example.com/rest@sha256:abc=def",
		testFlightImageParam: "registry.example.com/flight@sha256:123",
	}
	if err := mergeParamsEnv(fs, path, overrides); err != nil {
		t.Fatal(err)
	}

	got, err := fs.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	gotParams := make(map[string]string)
	scanner := bufio.NewScanner(strings.NewReader(string(got)))
	for scanner.Scan() {
		key, value, ok := strings.Cut(scanner.Text(), "=")
		if ok {
			gotParams[key] = value
		}
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	wantParams := map[string]string{
		testRestImageParam:   overrides[testRestImageParam],
		testFlightImageParam: overrides[testFlightImageParam],
		"KEEP":               "preserved",
	}
	if len(gotParams) != len(wantParams) {
		t.Fatalf("params.env entries = %#v, want %#v", gotParams, wantParams)
	}
	for key, want := range wantParams {
		if gotParams[key] != want {
			t.Errorf("params.env[%q] = %q, want %q", key, gotParams[key], want)
		}
	}
}

func TestMergeParamsEnvRejectsInvalidValuesWithoutWriting(t *testing.T) {
	fs := filesys.MakeFsInMemory()
	path := filepath.Join("/manifests", "params.env")
	if err := fs.MkdirAll(filepath.Dir(path)); err != nil {
		t.Fatal(err)
	}
	original := testRestImageParam + "=old:tag\n" + testFlightImageParam + "=old-flight:tag\n"
	if err := fs.WriteFile(path, []byte(original)); err != nil {
		t.Fatal(err)
	}

	err := mergeParamsEnv(fs, path, map[string]string{
		testRestImageParam:   "registry.example.com/rest:tag",
		testFlightImageParam: "registry.example.com/flight\nmalicious=value",
	})
	if err == nil {
		t.Fatal("expected error for a params.env value containing a newline")
	}

	got, readErr := fs.ReadFile(path)
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(got) != original {
		t.Errorf("params.env changed after failed merge: %q", got)
	}
}

func TestMergeParamsEnvRejectsUnknownKeyWithoutWriting(t *testing.T) {
	fs := filesys.MakeFsInMemory()
	path := filepath.Join("/manifests", "params.env")
	if err := fs.MkdirAll(filepath.Dir(path)); err != nil {
		t.Fatal(err)
	}
	original := "REST_IMAGE=old:tag\n"
	if err := fs.WriteFile(path, []byte(original)); err != nil {
		t.Fatal(err)
	}

	err := mergeParamsEnv(fs, path, map[string]string{
		"UNKNOWN_IMAGE": "registry.example.com/unknown:tag",
	})
	if err == nil {
		t.Fatal("expected error for an override key not present in params.env")
	}

	got, readErr := fs.ReadFile(path)
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(got) != original {
		t.Errorf("params.env changed after failed merge: %q", got)
	}
}

func TestAnnotateFlightDeploymentsWithConfigHash(t *testing.T) {
	configMap := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			testNameKey: "dch-default-dcs-flight-config",
			"labels": map[string]any{
				labelAppName: "default-dcs-flight",
			},
		},
		"data": map[string]any{
			"config.toml": "[connectors.uri]\nenabled = false\n",
		},
	}}
	deployment := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: "Deployment",
		testMetadataKey: map[string]any{
			testNameKey: "dch-default-dcs-flight",
		},
		"spec": map[string]any{
			"template": map[string]any{
				"spec": map[string]any{
					"containers": []any{
						map[string]any{
							testNameKey: "default-dcs-flight",
							"image":     "localhost/dch-flight:test",
						},
					},
				},
			},
		},
	}}

	annotateServiceDeploymentWithConfigHash([]*unstructured.Unstructured{configMap, deployment}, "default-dcs-flight")

	annotations, found, err := unstructured.NestedStringMap(
		deployment.Object,
		"spec",
		"template",
		testMetadataKey,
		"annotations",
	)
	if err != nil || !found {
		t.Fatalf("expected Flight Deployment template annotations, found=%v err=%v", found, err)
	}
	if annotations["dataconnecthub/config-hash"] == "" {
		t.Fatal("expected dataconnecthub/config-hash annotation on Flight Deployment")
	}
}
