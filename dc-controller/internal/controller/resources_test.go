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
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	"github.com/pelletier/go-toml/v2"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
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
	testDataKey                      = "data"
	testConfigTOMLKey                = "config.toml"
	testRestImageParam               = "REST_IMAGE"
	testFlightImageParam             = "FLIGHT_IMAGE"
	testSpecKey                      = "spec"
	testNamespace                    = "test-ns"
	testFlightInstanceName           = "default-dcs-flight"
	testFlightFQDN                   = "dch-" + testFlightInstanceName + "." + testNamespace + ".svc"
	testRestServiceAccount           = "system:serviceaccount:" + testNamespace + ":dch-rest-service-sa"
	testFlightServiceCA              = "flight-service-ca"
)

func labeledConfigMap(label, configTOML string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			"labels": map[string]any{
				labelAppName: label,
			},
		},
		testDataKey: map[string]any{
			testConfigTOMLKey: configTOML,
		},
	}}
}

func flightServiceConfigMap(configTOML string) *unstructured.Unstructured {
	return labeledConfigMap(nameFlightService, configTOML)
}

func restServiceConfigMap(configTOML string) *unstructured.Unstructured {
	return labeledConfigMap(nameRestService, configTOML)
}

func parsedConfigMapTOML(t *testing.T, configMap *unstructured.Unstructured) map[string]any {
	t.Helper()
	data, found, err := unstructured.NestedStringMap(configMap.Object, testDataKey)
	if err != nil || !found {
		t.Fatalf("expected ConfigMap data, found=%v err=%v", found, err)
	}
	var config map[string]any
	if err := toml.Unmarshal([]byte(data[testConfigTOMLKey]), &config); err != nil {
		t.Fatalf("parsing config.toml: %v", err)
	}
	return config
}

func TestInjectConfigOverlaysRestService(t *testing.T) {
	restCM := restServiceConfigMap(`[server]
port = 8080

[global-connection-types]
tenant-id = "opendatahub"

[flight-service]
address = "flight-service"
ca-cert = "/etc/tls/flight/ca.crt"
`)
	flightSvc := &unstructured.Unstructured{Object: map[string]any{
		testKindKey:     kindService,
		testMetadataKey: map[string]any{testNameKey: "dch-default-dcs-flight"},
	}}
	sa := &unstructured.Unstructured{Object: map[string]any{
		testKindKey:     kindServiceAccount,
		testMetadataKey: map[string]any{testNameKey: "dch-rest-service-sa"},
	}}

	cr := &dchv1alpha1.DataConnectService{}
	cr.SetName("default-dcs")
	cr.SetNamespace(testNamespace)
	params := (&DataConnectServiceReconciler{}).buildOverlayParams(cr, &platformConfig{}, []*unstructured.Unstructured{flightSvc, sa})
	if err := injectConfigOverlays([]*unstructured.Unstructured{restCM}, params); err != nil {
		t.Fatal(err)
	}

	config := parsedConfigMapTOML(t, restCM)

	global, ok := config["global-connection-types"].(map[string]any)
	if !ok {
		t.Fatal("expected [global-connection-types] table")
	}
	if got, want := global["tenant-id"], testNamespace; got != want {
		t.Errorf("tenant-id = %v, want %q", got, want)
	}

	fs, ok := config["flight-service"].(map[string]any)
	if !ok {
		t.Fatal("expected [flight-service] table")
	}
	if got, want := fs["address"], testFlightFQDN; got != want {
		t.Errorf("flight-service.address = %v, want %q", got, want)
	}
	if got := config["server"].(map[string]any)["port"]; got != int64(8080) {
		t.Errorf("unrelated server.port was changed; got %v", got)
	}
}

func TestInjectConfigOverlaysNoConfigMaps(t *testing.T) {
	if err := injectConfigOverlays(nil, overlayParams{Namespace: "ns", FlightInstanceName: "f"}); err != nil {
		t.Fatalf("expected nil error for empty resources, got %v", err)
	}
}

func TestInjectConfigOverlaysFlightService(t *testing.T) {
	flightCM := labeledConfigMap(testFlightInstanceName, `[server]
port = 8443

[global-connection-types]
tenant-id = "opendatahub"

[auth]
enabled = true

[connectors.default]
enabled = true
connection_timeout_secs = 10
`)
	flightCM.SetName("dch-" + nameFlightService + "-config")
	sa := &unstructured.Unstructured{Object: map[string]any{
		testKindKey:     kindServiceAccount,
		testMetadataKey: map[string]any{testNameKey: "dch-rest-service-sa"},
	}}
	audiences := []string{"aud-one", "aud-two"}

	cr := &dchv1alpha1.DataConnectService{}
	cr.SetName("default-dcs")
	cr.SetNamespace(testNamespace)
	cr.Spec.TokenReviewAudiences = audiences
	params := (&DataConnectServiceReconciler{}).buildOverlayParams(cr, &platformConfig{}, []*unstructured.Unstructured{sa})
	if err := injectConfigOverlays([]*unstructured.Unstructured{flightCM}, params); err != nil {
		t.Fatal(err)
	}

	config := parsedConfigMapTOML(t, flightCM)

	global, ok := config["global-connection-types"].(map[string]any)
	if !ok {
		t.Fatal("expected [global-connection-types] table")
	}
	if got, want := global["tenant-id"], testNamespace; got != want {
		t.Errorf("tenant-id = %v, want %q", got, want)
	}

	auth, ok := config["auth"].(map[string]any)
	if !ok {
		t.Fatal("expected [auth] table")
	}
	if got, want := auth["discovery_service_account"], testRestServiceAccount; got != want {
		t.Errorf("discovery_service_account = %v, want %q", got, want)
	}
	gotAudiences, ok := auth["token_review_audiences"].([]any)
	if !ok || len(gotAudiences) != len(audiences) {
		t.Fatalf("token_review_audiences = %#v, want %#v", auth["token_review_audiences"], audiences)
	}
	for i, want := range audiences {
		if gotAudiences[i] != want {
			t.Errorf("token_review_audiences[%d] = %v, want %q", i, gotAudiences[i], want)
		}
	}
}

func TestInjectConfigOverlaysConnectorOverrides(t *testing.T) {
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
			configTOML: `[connectors.default]
enabled = false
`,
			expected: "[connectors.sqlite]\nenabled = false",
		},
		{
			name:          "enabled custom connector",
			connectorName: "custom",
			enabled:       &enabled,
			configTOML: `[connectors.default]
enabled = false
`,
			expected: "[connectors.custom]\nenabled = true",
		},
		{
			name:          "missing enabled",
			connectorName: testSQLiteConnector,
			configTOML: `[connectors.default]
enabled = true
`,
			expected: "[connectors.sqlite]\nenabled = false",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			configMap := flightServiceConfigMap(tt.configTOML)

			inputs := overlayParams{
				Namespace:          testNamespace,
				FlightInstanceName: nameFlightService,
				Connectors:         []dchv1alpha1.ConnectorConfig{{Name: tt.connectorName, Enabled: tt.enabled}},
			}
			if err := injectConfigOverlays([]*unstructured.Unstructured{configMap}, inputs); err != nil {
				t.Fatal(err)
			}

			data, found, err := unstructured.NestedStringMap(configMap.Object, testDataKey)
			if err != nil || !found {
				t.Fatalf("expected ConfigMap data, found=%v err=%v", found, err)
			}
			if !strings.Contains(data[testConfigTOMLKey], tt.expected) {
				t.Fatalf("expected %s, got:\n%s", tt.expected, data[testConfigTOMLKey])
			}
		})
	}
}

func TestInjectConfigOverlaysConnectorTimeouts(t *testing.T) {
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

	inputs := overlayParams{
		Namespace:          testNamespace,
		FlightInstanceName: nameFlightService,
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
	}
	if err := injectConfigOverlays([]*unstructured.Unstructured{configMap}, inputs); err != nil {
		t.Fatal(err)
	}

	config := parsedConfigMapTOML(t, configMap)
	connectors, ok := config["connectors"].(map[string]any)
	if !ok {
		t.Fatalf("expected connectors table, got %#v", config["connectors"])
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

func TestInjectConfigOverlaysRejectsInvalidTOML(t *testing.T) {
	configMap := labeledConfigMap(nameFlightService, "[auth\nenabled = true")
	inputs := overlayParams{Namespace: testNamespace, FlightInstanceName: nameFlightService}
	if err := injectConfigOverlays([]*unstructured.Unstructured{configMap}, inputs); err == nil {
		t.Fatal("expected invalid TOML to return an error")
	}
}

func TestInjectConfigOverlaysSkipsUnlabeledConfigMaps(t *testing.T) {
	unlabeled := &unstructured.Unstructured{Object: map[string]any{
		testKindKey:     kindConfigMap,
		testMetadataKey: map[string]any{},
		testDataKey: map[string]any{
			testConfigTOMLKey: "[global-connection-types]\ntenant-id = \"opendatahub\"\n",
		},
	}}
	inputs := overlayParams{Namespace: testNamespace, FlightInstanceName: "flight"}
	if err := injectConfigOverlays([]*unstructured.Unstructured{unlabeled}, inputs); err != nil {
		t.Fatal(err)
	}
	global := parsedConfigMapTOML(t, unlabeled)["global-connection-types"].(map[string]any)
	if got := global["tenant-id"]; got != "opendatahub" {
		t.Errorf("unlabeled ConfigMap was modified; tenant-id = %v", got)
	}
}

func TestInjectConfigOverlaysOverridesCustomTenantID(t *testing.T) {
	configMap := restServiceConfigMap(`[global-connection-types]
tenant-id = "custom-tenant"
`)
	inputs := overlayParams{Namespace: testNamespace, FlightInstanceName: "flight"}
	if err := injectConfigOverlays([]*unstructured.Unstructured{configMap}, inputs); err != nil {
		t.Fatal(err)
	}
	global := parsedConfigMapTOML(t, configMap)["global-connection-types"].(map[string]any)
	if got := global["tenant-id"]; got != testNamespace {
		t.Errorf("tenant-id should always follow the CR namespace; got %v", got)
	}
}

func TestInjectConfigOverlaysFlightServiceAddressWithoutService(t *testing.T) {
	configMap := restServiceConfigMap(`[flight-service]
address = "flight-service"
`)
	inputs := overlayParams{Namespace: testNamespace, FlightInstanceName: testFlightInstanceName}
	if err := injectConfigOverlays([]*unstructured.Unstructured{configMap}, inputs); err != nil {
		t.Fatal(err)
	}
	fs := parsedConfigMapTOML(t, configMap)["flight-service"].(map[string]any)
	if got := fs["address"]; got != "flight-service" {
		t.Errorf("address should remain placeholder when no flight Service exists; got %v", got)
	}
}

func TestInjectConfigOverlaysForcesFlightServiceAddress(t *testing.T) {
	custom := restServiceConfigMap(`[flight-service]
address = "my-custom-flight.example.com"
ca-cert = "/etc/tls/flight/ca.crt"
`)
	missing := restServiceConfigMap(`[server]
port = 8080
`)
	inputs := overlayParams{
		Namespace:          testNamespace,
		FlightInstanceName: testFlightInstanceName,
		FlightFQDN:         testFlightFQDN,
	}
	if err := injectConfigOverlays([]*unstructured.Unstructured{custom, missing}, inputs); err != nil {
		t.Fatal(err)
	}

	fs := parsedConfigMapTOML(t, custom)["flight-service"].(map[string]any)
	if got, want := fs["address"], testFlightFQDN; got != want {
		t.Errorf("custom address should be overridden; got %v, want %q", got, want)
	}
	if got := fs["ca-cert"]; got != "/etc/tls/flight/ca.crt" {
		t.Errorf("existing flight-service keys should be preserved; ca-cert = %v", got)
	}

	fs, ok := parsedConfigMapTOML(t, missing)["flight-service"].(map[string]any)
	if !ok {
		t.Fatal("expected [flight-service] section to be created")
	}
	if got, want := fs["address"], testFlightFQDN; got != want {
		t.Errorf("address should be added when missing; got %v, want %q", got, want)
	}
}

func TestInjectConfigOverlaysCreatesAuthSection(t *testing.T) {
	flightCM := labeledConfigMap(testFlightInstanceName, `[server]
port = 8443
`)
	params := overlayParams{
		Namespace:          testNamespace,
		FlightInstanceName: testFlightInstanceName,
		RestServiceAccount: testRestServiceAccount,
		Audiences:          []string{"aud-one"},
	}
	if err := injectConfigOverlays([]*unstructured.Unstructured{flightCM}, params); err != nil {
		t.Fatal(err)
	}
	auth, ok := parsedConfigMapTOML(t, flightCM)["auth"].(map[string]any)
	if !ok {
		t.Fatal("expected [auth] section to be created")
	}
	if got, want := auth["discovery_service_account"], testRestServiceAccount; got != want {
		t.Errorf("discovery_service_account = %v, want %q", got, want)
	}
	if got := auth["token_review_audiences"]; got == nil {
		t.Error("expected token_review_audiences to be added")
	}
}

func TestMergeConfigOverlay(t *testing.T) {
	dst := map[string]any{
		"auth": map[string]any{
			"enabled":   true,
			"cache_ttl": int64(60),
		},
		"server": map[string]any{"port": int64(8080)},
	}
	overlay := map[string]any{
		"auth": map[string]any{
			"discovery_service_account": "system:serviceaccount:ns:sa",
		},
	}
	mergeConfigOverlay(dst, overlay)

	auth := dst["auth"].(map[string]any)
	if auth["enabled"] != true {
		t.Error("existing auth.enabled was clobbered")
	}
	if auth["cache_ttl"] != int64(60) {
		t.Error("existing auth.cache_ttl was clobbered")
	}
	if auth["discovery_service_account"] != "system:serviceaccount:ns:sa" {
		t.Error("overlay value was not merged")
	}
	if dst["server"].(map[string]any)["port"] != int64(8080) {
		t.Error("unrelated section was changed")
	}
}

func TestWholeSeconds(t *testing.T) {
	valid := &metav1.Duration{Duration: 30 * time.Second}
	got, err := wholeSeconds(valid, "connectionTimeout", "postgres")
	if err != nil {
		t.Fatal(err)
	}
	if *got != int64(30) {
		t.Errorf("wholeSeconds = %d, want 30", *got)
	}

	invalid := &metav1.Duration{Duration: 500 * time.Millisecond}
	if _, err := wholeSeconds(invalid, "connectionTimeout", "postgres"); err == nil {
		t.Fatal("expected error for sub-second duration")
	}

	got, err = wholeSeconds(nil, "connectionTimeout", "postgres")
	if err != nil || got != nil {
		t.Fatalf("nil duration should return nil, got %v err %v", got, err)
	}
}

func TestBuildServicePatchesUseSeparateDeploymentAndContainerNames(t *testing.T) {
	patches := buildServicePatches(nameFlightService, nameFlightServiceContainer, &dchv1alpha1.ServiceOverrides{
		Env: []corev1.EnvVar{{Name: "CUSTOM_VAR", Value: "custom-value"}},
	})
	if len(patches) != 1 {
		t.Fatalf("patch count = %d, want 1", len(patches))
	}
	if got := patches[0].Target.Name; got != nameFlightService {
		t.Errorf("patch target Deployment = %q, want %q", got, nameFlightService)
	}
	if !strings.Contains(patches[0].Patch, "metadata:\n  name: "+nameFlightService+"\n") {
		t.Errorf("patch does not target Deployment %q:\n%s", nameFlightService, patches[0].Patch)
	}
	if !strings.Contains(patches[0].Patch, "- name: "+nameFlightServiceContainer+"\n") {
		t.Errorf("patch does not target container %q:\n%s", nameFlightServiceContainer, patches[0].Patch)
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
				nameRestServiceContainer:   imageParams[RelatedImageRestService],
				nameFlightServiceContainer: imageParams[RelatedImageFlightService],
				nameKubeRbacProxy:          imageParams[RelatedImageKubeRbacProxy],
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
			if !ok || container[testNameKey] != containerName {
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
			testNameKey: "default-dcs-flight-config",
		},
		testDataKey: map[string]any{
			testConfigTOMLKey: "[connectors.uri]\nenabled = false\n",
		},
	}}
	deployment := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindDeployment,
		testMetadataKey: map[string]any{
			testNameKey: testFlightInstanceName,
		},
		testSpecKey: map[string]any{
			"template": map[string]any{
				testSpecKey: map[string]any{
					"containers": []any{
						map[string]any{
							testNameKey: nameFlightServiceContainer,
							"image":     "localhost/dch-flight:test",
						},
					},
					"volumes": []any{
						map[string]any{
							testNameKey: "config",
							"configMap": map[string]any{testNameKey: "default-dcs-flight-config"},
						},
						map[string]any{
							testNameKey: "tls",
							"secret":    map[string]any{"secretName": "default-dcs-flight-tls"},
						},
						map[string]any{
							testNameKey: "db-secret",
							"secret":    map[string]any{"secretName": nameDatabaseConfig},
						},
						map[string]any{
							testNameKey: "tmp",
							"emptyDir":  map[string]any{},
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

	// Verify hash changes when secret data changes.
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
		ObjectMeta: metav1.ObjectMeta{Name: testFlightServiceCA, Namespace: testNamespace},
		Data:       map[string]string{"service-ca.crt": "-----BEGIN CERTIFICATE-----\nMIIC...\n-----END CERTIFICATE-----\n"},
	}
	fakeClient := fake.NewClientBuilder().WithScheme(s).WithObjects(liveCA).Build()
	r := &DataConnectServiceReconciler{Client: fakeClient}

	emptyCA := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			testNameKey: testFlightServiceCA,
		},
		testDataKey: map[string]any{},
	}}
	deployment := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindDeployment,
		testMetadataKey: map[string]any{
			testNameKey: nameRestService,
		},
		testSpecKey: map[string]any{
			"template": map[string]any{
				testSpecKey: map[string]any{
					"containers": []any{
						map[string]any{testNameKey: nameRestServiceContainer, "image": "test:latest"},
					},
					"volumes": []any{
						map[string]any{
							testNameKey: "flight-ca",
							"configMap": map[string]any{testNameKey: testFlightServiceCA},
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
