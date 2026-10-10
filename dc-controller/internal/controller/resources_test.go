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
	testFlightServiceCA              = "flight-service-ca"
)

func serviceConfigMap(serviceName, configTOML string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			testNameKey: renderedName(serviceName + "-config"),
			"labels": map[string]any{
				"app.kubernetes.io/name": serviceName,
			},
		},
		testDataKey: map[string]any{
			testConfigTOMLKey: configTOML,
		},
	}}
}

// testCR mirrors the singleton CR the controller reconciles; its name feeds
// flightServiceResourceName, so the derived flight instance name is
// "default-dcs-flight".
func testCR() *dchv1alpha1.DataConnectService {
	return &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: "default-dcs", Namespace: testNamespace},
	}
}

func testCRWithFlightOverrides(overrides dchv1alpha1.ServiceOverrides) *dchv1alpha1.DataConnectService {
	cr := testCR()
	cr.Spec.FlightService = &dchv1alpha1.FlightServiceConfig{ServiceOverrides: overrides}
	return cr
}

func flightServiceConfigMap(configTOML string) *unstructured.Unstructured {
	return serviceConfigMap(flightServiceResourceName("default-dcs"), configTOML)
}

func restServiceConfigMap(configTOML string) *unstructured.Unstructured {
	return serviceConfigMap(nameRestService, configTOML)
}

func namedConfigMap(name, configTOML string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			testNameKey: name,
		},
		testDataKey: map[string]any{
			testConfigTOMLKey: configTOML,
		},
	}}
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

// flightService returns the rendered flight-service Service with the
// post-rename name for the default test CR.
func flightService() *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindService,
		testMetadataKey: map[string]any{
			testNameKey: "dch-default-dcs-flight",
		},
	}}
}

// restServiceAccount returns the rendered rest-service ServiceAccount.
func restServiceAccount() *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindServiceAccount,
		testMetadataKey: map[string]any{
			testNameKey: "dch-rest-service-sa",
		},
	}}
}

func TestReconcileFlightConfigMapTOMLAddConnector(t *testing.T) {
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
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			configMap := flightServiceConfigMap(tt.configTOML)

			if err := reconcileFlightConfigMapTOML([]*unstructured.Unstructured{restServiceAccount(), configMap}, testCRWithFlightOverrides(dchv1alpha1.ServiceOverrides{
				Connectors: []dchv1alpha1.ConnectorConfig{{Name: tt.connectorName, Enabled: tt.enabled}},
			}), nil); err != nil {
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

func TestReconcileFlightConfigMapTOMLEnabledOmitted(t *testing.T) {
	// When the CR omits enabled, an existing value is preserved; a
	// connector whose settings are all omitted gets an empty section,
	// which the service treats as a no-op override.
	configMap := flightServiceConfigMap(`[connectors.default]
enabled = true

[connectors.s3]
enabled = true
chunk_size = 1024
`)

	if err := reconcileFlightConfigMapTOML([]*unstructured.Unstructured{restServiceAccount(), configMap}, testCRWithFlightOverrides(dchv1alpha1.ServiceOverrides{
		Connectors: []dchv1alpha1.ConnectorConfig{
			{Name: "s3", ConnectionTimeout: &metav1.Duration{Duration: 30 * time.Second}},
			{Name: testSQLiteConnector},
		},
	}), nil); err != nil {
		t.Fatal(err)
	}

	connectors, ok := parsedConfigMapTOML(t, configMap)["connectors"].(map[string]any)
	if !ok {
		t.Fatal("expected connectors table")
	}
	s3, ok := connectors["s3"].(map[string]any)
	if !ok {
		t.Fatal("expected connectors.s3 table")
	}
	if got := s3[testTOMLEnabledKey]; got != true {
		t.Errorf("connectors.s3.enabled = %v, want true (existing value must be preserved)", got)
	}
	if got := s3[testTOMLConnectionTimeoutSecsKey]; got != int64(30) {
		t.Errorf("connectors.s3.connection_timeout_secs = %v, want 30", got)
	}
	if got := s3["chunk_size"]; got != int64(1024) {
		t.Errorf("connectors.s3.chunk_size = %v, want 1024 (unrelated setting must be preserved)", got)
	}
	sqlite, ok := connectors[testSQLiteConnector].(map[string]any)
	if !ok {
		t.Fatalf("expected connectors.sqlite table, got %#v", connectors[testSQLiteConnector])
	}
	if len(sqlite) != 0 {
		t.Errorf("connectors.sqlite = %#v, want an empty section when all settings are omitted", sqlite)
	}
}

func TestReconcileFlightConfigMapTOMLUpdateConnector(t *testing.T) {
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

	if err := reconcileFlightConfigMapTOML([]*unstructured.Unstructured{restServiceAccount(), configMap}, testCRWithFlightOverrides(dchv1alpha1.ServiceOverrides{
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
	}), nil); err != nil {
		t.Fatal(err)
	}

	data, found, err := unstructured.NestedStringMap(configMap.Object, testDataKey)
	if err != nil || !found {
		t.Fatalf("expected ConfigMap data, found=%v err=%v", found, err)
	}
	var updated map[string]any
	if err := toml.Unmarshal([]byte(data[testConfigTOMLKey]), &updated); err != nil {
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

func TestReconcileRestConfigMapTOMLFlightServiceAddress(t *testing.T) {
	configMap := restServiceConfigMap(`[flight-service]
address = "flight-service"
`)
	service := flightService()

	if err := reconcileRestConfigMapTOML([]*unstructured.Unstructured{service, configMap}, testCR()); err != nil {
		t.Fatal(err)
	}
	flightService, ok := parsedConfigMapTOML(t, configMap)["flight-service"].(map[string]any)
	if !ok {
		t.Fatal("expected [flight-service] table")
	}
	if got, want := flightService["address"], "dch-default-dcs-flight."+testNamespace+".svc"; got != want {
		t.Errorf("flight-service.address = %v, want %q", got, want)
	}
}

func TestReconcileConfigMapTOMLGlobalNamespace(t *testing.T) {
	// tenant-id is always overwritten with the CR namespace, and a missing
	// [global-connection-types] section is created.
	for _, tt := range []struct {
		name       string
		configTOML string
		flight     bool
	}{
		{name: "flight default tenant", configTOML: "[global-connection-types]\ntenant-id = \"opendatahub\"\n", flight: true},
		{name: "rest default tenant", configTOML: "[global-connection-types]\ntenant-id = \"opendatahub\"\n"},
		{name: "flight custom tenant is overwritten", configTOML: "[global-connection-types]\ntenant-id = \"custom-tenant\"\n", flight: true},
		{name: "rest custom tenant is overwritten", configTOML: "[global-connection-types]\ntenant-id = \"custom-tenant\"\n"},
		{name: "flight missing section is created", configTOML: "[server]\nport = 8443\n", flight: true},
		{name: "rest missing section is created", configTOML: "[server]\nport = 8080\n"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var (
				cm  *unstructured.Unstructured
				err error
			)
			if tt.flight {
				cm = flightServiceConfigMap(tt.configTOML)
				err = reconcileFlightConfigMapTOML([]*unstructured.Unstructured{restServiceAccount(), cm}, testCR(), nil)
			} else {
				cm = restServiceConfigMap(tt.configTOML)
				err = reconcileRestConfigMapTOML([]*unstructured.Unstructured{flightService(), cm}, testCR())
			}
			if err != nil {
				t.Fatal(err)
			}
			global, ok := parsedConfigMapTOML(t, cm)["global-connection-types"].(map[string]any)
			if !ok {
				t.Fatal("expected [global-connection-types] table")
			}
			if got := global["tenant-id"]; got != testNamespace {
				t.Errorf("tenant-id = %v, want %q", got, testNamespace)
			}
		})
	}

	// A ConfigMap with an unrecognized name is left untouched.
	unknownConfigMap := namedConfigMap("dch-other-service-config", `[global-connection-types]
tenant-id = "opendatahub"
`)
	resources := []*unstructured.Unstructured{
		restServiceAccount(),
		flightService(),
		flightServiceConfigMap("[server]\nport = 8443\n"),
		restServiceConfigMap("[server]\nport = 8080\n"),
		unknownConfigMap,
	}
	if err := reconcileFlightConfigMapTOML(resources, testCR(), nil); err != nil {
		t.Fatal(err)
	}
	if err := reconcileRestConfigMapTOML(resources, testCR()); err != nil {
		t.Fatal(err)
	}
	if got := parsedConfigMapTOML(t, unknownConfigMap)["global-connection-types"].(map[string]any)["tenant-id"]; got != "opendatahub" {
		t.Errorf("ConfigMap with an unrecognized name was changed unexpectedly; tenant-id = %v", got)
	}
}

func TestReconcileFlightConfigMapTOMLDiscoveryServiceAccount(t *testing.T) {
	// The [auth] section is created when absent, and the discovery service
	// account is always the REST service account identity.
	for _, tt := range []struct {
		name       string
		configTOML string
	}{
		{
			name:       "existing auth section",
			configTOML: "[auth]\nenabled = true\ndiscovery_service_account = \"old-identity\"\n",
		},
		{
			name:       "missing auth section is created",
			configTOML: "[server]\nport = 8443\n",
		},
	} {
		t.Run(tt.name, func(t *testing.T) {
			configMap := flightServiceConfigMap(tt.configTOML)

			if err := reconcileFlightConfigMapTOML([]*unstructured.Unstructured{restServiceAccount(), configMap}, testCR(), nil); err != nil {
				t.Fatal(err)
			}
			auth, ok := parsedConfigMapTOML(t, configMap)["auth"].(map[string]any)
			if !ok {
				t.Fatal("expected [auth] table")
			}
			if got, want := auth["discovery_service_account"], "system:serviceaccount:"+testNamespace+":dch-rest-service-sa"; got != want {
				t.Errorf("discovery_service_account = %v, want %q", got, want)
			}
		})
	}
}

func TestReconcileFlightConfigMapTOMLAudiences(t *testing.T) {
	configMap := flightServiceConfigMap(`[auth]
enabled = true
token_review_audiences = ["old-audience"]
`)
	noAuth := restServiceConfigMap(`[server]
port = 8080
`)
	audiences := []string{"audience-one", "audience-two"}

	if err := reconcileFlightConfigMapTOML([]*unstructured.Unstructured{restServiceAccount(), configMap, noAuth}, testCR(), audiences); err != nil {
		t.Fatal(err)
	}
	auth, ok := parsedConfigMapTOML(t, configMap)["auth"].(map[string]any)
	if !ok {
		t.Fatal("expected [auth] table")
	}
	got, ok := auth["token_review_audiences"].([]any)
	if !ok || len(got) != len(audiences) {
		t.Fatalf("token_review_audiences = %#v, want %#v", auth["token_review_audiences"], audiences)
	}
	for i, audience := range audiences {
		if got[i] != audience {
			t.Errorf("token_review_audiences[%d] = %v, want %q", i, got[i], audience)
		}
	}
	if _, auth := parsedConfigMapTOML(t, noAuth)["auth"]; auth {
		t.Error("expected no [auth] section to be added to the rest-service ConfigMap")
	}
	if got := parsedConfigMapTOML(t, noAuth)["server"].(map[string]any)["port"]; got != int64(8080) {
		t.Errorf("ConfigMap without [auth] was changed unexpectedly; server.port = %v", got)
	}
}

func TestReconcileFlightConfigMapTOMLRejectsInvalidTOML(t *testing.T) {
	configMap := flightServiceConfigMap("[auth\nenabled = true")
	if err := reconcileFlightConfigMapTOML([]*unstructured.Unstructured{configMap}, testCR(), []string{"audience"}); err == nil {
		t.Fatal("expected invalid TOML to return an error")
	}
}

func TestReconcileConfigMapTOMLMissingConfigMapAndToml(t *testing.T) {
	// The base manifests always render both ConfigMaps with a config.toml;
	// their absence means the manifests drifted, which must fail loudly
	// instead of silently skipping the CR-driven configuration.
	if err := reconcileFlightConfigMapTOML(nil, testCR(), nil); err == nil {
		t.Fatal("expected an error when the flight-service ConfigMap is missing")
	}
	if err := reconcileRestConfigMapTOML(nil, testCR()); err == nil {
		t.Fatal("expected an error when the rest-service ConfigMap is missing")
	}

	flightNoToml := flightServiceConfigMap("")
	delete(flightNoToml.Object[testDataKey].(map[string]any), testConfigTOMLKey)
	if err := reconcileFlightConfigMapTOML([]*unstructured.Unstructured{flightNoToml}, testCR(), nil); err == nil {
		t.Fatal("expected an error when the flight-service ConfigMap has no config.toml")
	}

	restNoToml := restServiceConfigMap("")
	delete(restNoToml.Object[testDataKey].(map[string]any), testConfigTOMLKey)
	if err := reconcileRestConfigMapTOML([]*unstructured.Unstructured{restNoToml}, testCR()); err == nil {
		t.Fatal("expected an error when the rest-service ConfigMap has no config.toml")
	}
}

func TestReconcileConfigMapTOMLAppliesAllSettings(t *testing.T) {
	flightConfigMap := flightServiceConfigMap(`[global-connection-types]
tenant-id = "opendatahub"

[auth]
enabled = true

[connectors.default]
enabled = true
`)
	restConfigMap := restServiceConfigMap(`[server]
port = 8080

[global-connection-types]
tenant-id = "opendatahub"

[flight-service]
address = "flight-service"
`)
	service := flightService()
	serviceAccount := restServiceAccount()

	enabled := true
	audiences := []string{"audience-one", "audience-two"}
	cr := testCRWithFlightOverrides(dchv1alpha1.ServiceOverrides{
		Connectors: []dchv1alpha1.ConnectorConfig{{Name: testSQLiteConnector, Enabled: &enabled}},
	})

	if err := reconcileFlightConfigMapTOML(
		[]*unstructured.Unstructured{service, serviceAccount, flightConfigMap, restConfigMap},
		cr, audiences,
	); err != nil {
		t.Fatal(err)
	}
	if err := reconcileRestConfigMapTOML(
		[]*unstructured.Unstructured{service, serviceAccount, flightConfigMap, restConfigMap},
		testCR(),
	); err != nil {
		t.Fatal(err)
	}

	flightConfig := parsedConfigMapTOML(t, flightConfigMap)
	if got := flightConfig["global-connection-types"].(map[string]any)["tenant-id"]; got != testNamespace {
		t.Errorf("flight tenant-id = %v, want %q", got, testNamespace)
	}
	auth := flightConfig["auth"].(map[string]any)
	if got, want := auth["discovery_service_account"], "system:serviceaccount:"+testNamespace+":dch-rest-service-sa"; got != want {
		t.Errorf("discovery_service_account = %v, want %q", got, want)
	}
	got, ok := auth["token_review_audiences"].([]any)
	if !ok || len(got) != len(audiences) {
		t.Fatalf("token_review_audiences = %#v, want %#v", auth["token_review_audiences"], audiences)
	}
	for i, audience := range audiences {
		if got[i] != audience {
			t.Errorf("token_review_audiences[%d] = %v, want %q", i, got[i], audience)
		}
	}
	connectors := flightConfig["connectors"].(map[string]any)
	if got := connectors[testSQLiteConnector].(map[string]any)["enabled"]; got != true {
		t.Errorf("connectors.sqlite.enabled = %v, want true", got)
	}
	if got := connectors["default"].(map[string]any)["enabled"]; got != true {
		t.Errorf("connectors.default.enabled = %v, want true (unrelated connector must be preserved)", got)
	}
	if _, exists := flightConfig["flight-service"]; exists {
		t.Error("expected no [flight-service] section in the flight-service ConfigMap")
	}

	restConfig := parsedConfigMapTOML(t, restConfigMap)
	if got := restConfig["global-connection-types"].(map[string]any)["tenant-id"]; got != testNamespace {
		t.Errorf("rest tenant-id = %v, want %q", got, testNamespace)
	}
	wantFQDN := "dch-default-dcs-flight." + testNamespace + ".svc"
	if got := restConfig["flight-service"].(map[string]any)["address"]; got != wantFQDN {
		t.Errorf("flight-service.address = %v, want %q", got, wantFQDN)
	}
	if _, exists := restConfig["auth"]; exists {
		t.Error("expected no [auth] section in the rest-service ConfigMap")
	}
	if _, exists := restConfig["connectors"]; exists {
		t.Error("expected no [connectors] section in the rest-service ConfigMap")
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
			testNameKey: "default-dcs-flight",
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
