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
	"slices"
	"strings"
	"testing"
	"time"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	platformtls "github.com/opendatahub-io/odh-platform-utilities/framework/tls"
	configv1 "github.com/openshift/api/config/v1"
	"github.com/pelletier/go-toml/v2"
	corev1 "k8s.io/api/core/v1"
	networkingv1 "k8s.io/api/networking/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
	"sigs.k8s.io/controller-runtime/pkg/client/interceptor"
	"sigs.k8s.io/kustomize/kyaml/filesys"
)

const (
	testBaseManifest                 = "base"
	testControllerNamespace          = "controller-ns"
	testGatewayName                  = "custom-gateway"
	testOpenShiftGatewayNamespace    = "openshift-ingress"
	testOperandNamespace             = "operand-ns"
	testSQLiteConnector              = "sqlite"
	testTOMLEnabledKey               = "enabled"
	testTOMLConnectionTimeoutSecsKey = "connection_timeout_secs"
	testTOMLRequestTimeoutSecsKey    = "request_timeout_secs"
	testTOMLReadTimeoutSecsKey       = "read_timeout_secs"
	testKindKey                      = "kind"
	testMetadataKey                  = "metadata"
	testDataKey                      = "data"
	testTemplateKey                  = "template"
	testContainersKey                = "containers"
	testConfigTOMLKey                = "config.toml"
	testRestImageParam               = "REST_IMAGE"
	testFlightImageParam             = "FLIGHT_IMAGE"
	testSpecKey                      = "spec"
	testNamespace                    = "test-ns"
	testFlightServiceCA              = "flight-service-ca"
)

func flightServiceConfigMap(configTOML string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindConfigMap,
		testMetadataKey: map[string]any{
			"labels": map[string]any{
				"app.kubernetes.io/name": nameFlightService,
			},
		},
		testDataKey: map[string]any{
			testConfigTOMLKey: configTOML,
		},
	}}
}

func configMapWithTOML(configTOML string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey:     kindConfigMap,
		testMetadataKey: map[string]any{},
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

func TestSetConfigMapFlightServiceAddress(t *testing.T) {
	configMap := configMapWithTOML(`[flight-service]
address = "flight-service"
`)
	service := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindService,
		"metadata": map[string]any{
			resourceNameField: "dch-default-dcs-flight",
		},
	}}

	if err := setConfigMapFlightServiceAddress([]*unstructured.Unstructured{service, configMap}, "test-namespace", "default-dcs-flight"); err != nil {
		t.Fatal(err)
	}
	flightService, ok := parsedConfigMapTOML(t, configMap)["flight-service"].(map[string]any)
	if !ok {
		t.Fatal("expected [flight-service] table")
	}
	if got, want := flightService["address"], "dch-default-dcs-flight.test-namespace.svc"; got != want {
		t.Errorf("flight-service.address = %v, want %q", got, want)
	}
}

func TestSetConfigMapGlobalNamespace(t *testing.T) {
	defaultTenant := configMapWithTOML(`[global-connection-types]
tenant-id = "opendatahub"
`)
	customTenant := configMapWithTOML(`[global-connection-types]
tenant-id = "custom-tenant"
`)
	noTenant := configMapWithTOML(`[server]
port = 8080
`)

	if err := setConfigMapGlobalNamespace([]*unstructured.Unstructured{defaultTenant, customTenant, noTenant}, "test-namespace"); err != nil {
		t.Fatal(err)
	}

	for _, tt := range []struct {
		name string
		cm   *unstructured.Unstructured
		want string
	}{
		{name: "default tenant", cm: defaultTenant, want: "test-namespace"},
		{name: "custom tenant remains unchanged", cm: customTenant, want: "custom-tenant"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			global, ok := parsedConfigMapTOML(t, tt.cm)["global-connection-types"].(map[string]any)
			if !ok {
				t.Fatal("expected [global-connection-types] table")
			}
			if got := global["tenant-id"]; got != tt.want {
				t.Errorf("tenant-id = %v, want %q", got, tt.want)
			}
		})
	}
	if got := parsedConfigMapTOML(t, noTenant)["server"].(map[string]any)["port"]; got != int64(8080) {
		t.Errorf("ConfigMap without tenant-id was changed unexpectedly; server.port = %v", got)
	}
}

func TestSetConfigMapDiscoveryServiceAccount(t *testing.T) {
	configMap := configMapWithTOML(`[auth]
enabled = true
discovery_service_account = "old-identity"
`)
	configMap.SetName("dch-default-dcs-flight-config")
	serviceAccount := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindServiceAccount,
		"metadata": map[string]any{
			resourceNameField: "dch-rest-service-sa",
		},
	}}

	if err := setConfigMapDiscoveryServiceAccount([]*unstructured.Unstructured{serviceAccount, configMap}, "test-namespace", "default-dcs-flight"); err != nil {
		t.Fatal(err)
	}
	auth, ok := parsedConfigMapTOML(t, configMap)["auth"].(map[string]any)
	if !ok {
		t.Fatal("expected [auth] table")
	}
	if got, want := auth["discovery_service_account"], "system:serviceaccount:test-namespace:dch-rest-service-sa"; got != want {
		t.Errorf("discovery_service_account = %v, want %q", got, want)
	}
}

func TestSetConfigMapAudiences(t *testing.T) {
	configMap := configMapWithTOML(`[auth]
enabled = true
token_review_audiences = ["old-audience"]
`)
	noAuth := configMapWithTOML(`[server]
port = 8080
`)
	audiences := []string{"audience-one", "audience-two"}

	updated, err := setConfigMapAudiences([]*unstructured.Unstructured{configMap, noAuth}, audiences)
	if err != nil {
		t.Fatal(err)
	}
	if !updated {
		t.Fatal("expected an auth ConfigMap to be updated")
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
	if got := parsedConfigMapTOML(t, noAuth)["server"].(map[string]any)["port"]; got != int64(8080) {
		t.Errorf("ConfigMap without [auth] was changed unexpectedly; server.port = %v", got)
	}
}

func TestSetConfigMapAudiencesRejectsInvalidTOML(t *testing.T) {
	configMap := configMapWithTOML("[auth\nenabled = true")
	if _, err := setConfigMapAudiences([]*unstructured.Unstructured{configMap}, []string{"audience"}); err == nil {
		t.Fatal("expected invalid TOML to return an error")
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
		{name: testBaseManifest, path: filepath.Join(manifestsPath, testBaseManifest)},
		{name: "openshift overlay", path: filepath.Join(manifestsPath, "overlays", "openshift"), wantServiceMonitor: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			resources, err := renderKustomization(manifestsPath, tt.path, nil, nil)
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

func TestRenderKustomizationGatewayNamespace(t *testing.T) {
	root := filepath.Join("..", "..", "..", "config")
	for _, path := range []string{testBaseManifest, filepath.Join("overlays", "openshift")} {
		t.Run(path, func(t *testing.T) {
			resources, err := renderKustomization(root, filepath.Join(root, path), nil, map[string]string{
				"CONTROLLER_NAMESPACE": testControllerNamespace,
				"GATEWAY_NAME":         testGatewayName,
				"GATEWAY_NAMESPACE":    testOpenShiftGatewayNamespace,
				"OPERAND_NAMESPACE":    testOperandNamespace,
			})
			if err != nil {
				t.Fatal(err)
			}

			found := 0
			paramsFound := false
			for _, obj := range resources {
				if obj.GetKind() == kindConfigMap && obj.GetName() == "dch-params" {
					paramsFound = true
				}
				if obj.GetKind() != kindNetworkPolicy {
					continue
				}
				var policy networkingv1.NetworkPolicy
				if err := runtime.DefaultUnstructuredConverter.FromUnstructured(obj.Object, &policy); err != nil {
					t.Fatal(err)
				}
				gatewayIngress := 0
				if policy.Labels[labelAppName] == nameFlightService {
					gatewayIngress = 1
					peer := policy.Spec.Ingress[0].From[0]
					if got := peer.NamespaceSelector.MatchLabels["kubernetes.io/metadata.name"]; got != testOperandNamespace {
						t.Errorf("%s REST namespace = %q, want %s", policy.Name, got, testOperandNamespace)
					}
					if peer.PodSelector.MatchLabels[labelAppName] != nameRestService {
						t.Errorf("%s REST selector = %q, want %s", policy.Name, peer.PodSelector.MatchLabels[labelAppName], nameRestService)
					}
				} else {
					peer := policy.Spec.Ingress[1].From[0]
					if got := peer.NamespaceSelector.MatchLabels["kubernetes.io/metadata.name"]; got != testControllerNamespace {
						t.Errorf("%s controller namespace = %q, want %s", policy.Name, got, testControllerNamespace)
					}
					if peer.PodSelector.MatchLabels[labelAppName] != "dc-controller" {
						t.Errorf("%s controller selector = %q, want dc-controller", policy.Name, peer.PodSelector.MatchLabels[labelAppName])
					}
				}
				got := policy.Spec.Ingress[gatewayIngress].From[0].NamespaceSelector.MatchLabels["kubernetes.io/metadata.name"]
				if got != testOpenShiftGatewayNamespace {
					t.Errorf("%s gateway namespace = %q, want %s", policy.Name, got, testOpenShiftGatewayNamespace)
				}
				got = policy.Spec.Ingress[gatewayIngress].From[0].PodSelector.MatchLabels["gateway.networking.k8s.io/gateway-name"]
				if got != testGatewayName {
					t.Errorf("%s gateway name = %q, want %s", policy.Name, got, testGatewayName)
				}
				found++
			}
			if found != 2 {
				t.Errorf("rendered %d NetworkPolicies, want 2", found)
			}
			if !paramsFound {
				t.Error("rendered params ConfigMap must have a stable name")
			}
		})
	}
}

func TestSetKubeRbacProxyTLSOptsProjectsStrictProfiles(t *testing.T) {
	tests := []struct {
		name     string
		profile  configv1.TLSProfileSpec
		wantMin  string
		wantSuit string
	}{
		{
			name:    "Modern profile only sets TLS 1.3 minimum",
			profile: *configv1.TLSProfiles[configv1.TLSProfileModernType],
			wantMin: "--tls-min-version=VersionTLS13",
		},
		{
			name:     "Intermediate profile sets minimum and IANA cipher suites",
			profile:  *configv1.TLSProfiles[configv1.TLSProfileIntermediateType],
			wantMin:  "--tls-min-version=VersionTLS12",
			wantSuit: "--tls-cipher-suites=TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			resources, deployment := renderedRESTResources(t)
			args := kubeRbacProxyArgs(t, deployment)
			args = append(args, "--tls-min-version", "old", "--tls-cipher-suites", "old")
			setKubeRbacProxyArgs(t, deployment, args)
			setKubeRbacProxyTLSOpts(context.Background(), resources, tt.profile)
			setKubeRbacProxyTLSOpts(context.Background(), resources, tt.profile)
			args = kubeRbacProxyArgs(t, deployment)
			if !containsString(args, tt.wantMin) {
				t.Errorf("args %v do not contain %q", args, tt.wantMin)
			}
			if tt.wantSuit != "" && !containsString(args, tt.wantSuit) {
				t.Errorf("args %v do not contain %q", args, tt.wantSuit)
			}
			if tt.wantSuit == "" && containsArgPrefix(args, "--tls-cipher-suites=") {
				t.Errorf("TLS 1.3 args unexpectedly include cipher suites: %v", args)
			}
			if containsString(args, "--tls-min-version") || containsString(args, "--tls-cipher-suites") || containsString(args, "old") {
				t.Errorf("stale TLS flags or values remain in args: %v", args)
			}
			if countArgPrefix(args, "--tls-min-version=") != 1 || countArgPrefix(args, "--tls-cipher-suites=") > 1 {
				t.Errorf("TLS flags were not replaced idempotently: %v", args)
			}
		})
	}
}

func TestSetKubeRbacProxyTLSOptsProjectsLenientProfiles(t *testing.T) {
	// kube-rbac-proxy is an adhering component the controller owns, so
	// reconcileManifests resolves the cluster profile with LoadWithAdherence
	// and projects it even under lenient adherence policies. Mirror that
	// composition so a regression that skips projection or resolves a
	// different profile under lenient policies fails here.
	for _, policy := range []configv1.TLSAdherencePolicy{
		configv1.TLSAdherencePolicyNoOpinion,
		configv1.TLSAdherencePolicyLegacyAdheringComponentsOnly,
	} {
		t.Run(string(policy), func(t *testing.T) {
			apiServer := &configv1.APIServer{
				ObjectMeta: metav1.ObjectMeta{Name: "cluster"},
				Spec: configv1.APIServerSpec{
					TLSSecurityProfile: &configv1.TLSSecurityProfile{Type: configv1.TLSProfileModernType},
					TLSAdherence:       policy,
				},
			}
			profile, err := platformtls.LoadWithAdherence(context.Background(), tlsConfigClient(t, apiServer))
			if err != nil {
				t.Fatalf("LoadWithAdherence() under %s: %v", policy, err)
			}
			deployment := kubeRbacProxyDeployment("--secure-listen-address=0.0.0.0:8443", "--tls-min-version=custom", "--tls-cipher-suites=custom")
			setKubeRbacProxyTLSOpts(context.Background(), []*unstructured.Unstructured{deployment}, profile.Spec)
			args := kubeRbacProxyArgs(t, deployment)
			if !containsString(args, "--tls-min-version=VersionTLS13") || containsArgPrefix(args, "--tls-cipher-suites=") || containsString(args, "custom") {
				t.Fatalf("args = %v, want projected Modern profile without stale TLS flags", args)
			}
		})
	}
}

func TestSetKubeRbacProxyTLSOptsFallsBackToIntermediateWhenAPIServerObjectIsAbsent(t *testing.T) {
	// The scheme registers the OpenShift API, so the missing object exercises
	// the NotFound fallback specifically (someone deleted the APIServer
	// object on a real cluster). The genuinely-unavailable-API case needs the
	// RESTMapper to report NoMatch and is covered end to end by the envtest
	// suite, which runs on vanilla Kubernetes.
	profile, err := platformtls.LoadWithAdherence(context.Background(), tlsConfigClient(t))
	if err != nil {
		t.Fatalf("LoadWithAdherence() without APIServer: %v", err)
	}
	if !profile.UsedFallback {
		t.Fatal("expected the Intermediate fallback when the APIServer object is absent")
	}
	deployment := kubeRbacProxyDeployment("--secure-listen-address=0.0.0.0:8443")
	setKubeRbacProxyTLSOpts(context.Background(), []*unstructured.Unstructured{deployment}, profile.Spec)
	args := kubeRbacProxyArgs(t, deployment)
	if !containsString(args, "--tls-min-version=VersionTLS12") || !containsArgPrefix(args, "--tls-cipher-suites=TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256") {
		t.Fatalf("args = %v, want projected Intermediate fallback", args)
	}
}

func TestReconcileManifestsFailsClosedWhenTLSProfileReadDenied(t *testing.T) {
	// A denied APIServer read must fail the reconcile before any resources
	// are applied, so a misconfigured RBAC surface cannot roll out proxy
	// pods with stale or missing TLS options.
	cli := fake.NewClientBuilder().
		WithScheme(tlsConfigScheme(t)).
		WithInterceptorFuncs(interceptor.Funcs{
			Get: func(ctx context.Context, c client.WithWatch, key client.ObjectKey, obj client.Object, opts ...client.GetOption) error {
				if _, ok := obj.(*configv1.APIServer); ok {
					return apierrors.NewForbidden(schema.GroupResource{Group: "config.openshift.io", Resource: "apiservers"}, key.Name, nil)
				}
				return c.Get(ctx, key, obj, opts...)
			},
		}).Build()
	r := &DataConnectServiceReconciler{
		Client:             cli,
		ManifestsPath:      filepath.Join("..", "..", "..", "config"),
		RestImage:          "example.com/rest:latest",
		FlightImage:        "example.com/flight:latest",
		KubeRbacProxyImage: "example.com/kube-rbac-proxy:latest",
	}
	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: "test-dcs", Namespace: testNamespace},
	}
	platCfg := &platformConfig{GatewayName: "test-gateway", GatewayNamespace: testNamespace}

	err := r.reconcileManifests(context.Background(), cr, platCfg)
	if err == nil {
		t.Fatal("reconcileManifests() = nil error, want fail-closed error on denied APIServer read")
	}
	if !strings.Contains(err.Error(), "resolving cluster TLS profile for kube-rbac-proxy") {
		t.Fatalf("reconcileManifests() error = %v, want the TLS profile resolution error", err)
	}
}

// tlsConfigClient returns a fake client serving the OpenShift config API with
// the given objects, matching what reconcileManifests reads through
// platformtls.LoadWithAdherence.
func tlsConfigClient(t *testing.T, objects ...client.Object) client.Client {
	t.Helper()
	return fake.NewClientBuilder().
		WithScheme(tlsConfigScheme(t)).
		WithObjects(objects...).
		Build()
}

func tlsConfigScheme(t *testing.T) *runtime.Scheme {
	t.Helper()
	scheme := runtime.NewScheme()
	if err := configv1.Install(scheme); err != nil {
		t.Fatalf("installing OpenShift config scheme: %v", err)
	}
	return scheme
}

func TestRenderRESTServiceTLSLockdown(t *testing.T) {
	resources := renderRESTBase(t)
	configMap, deployment, networkPolicy := findRESTServiceResources(t, resources)
	assertRESTServiceConfig(t, configMap)
	assertRESTServiceProbes(t, deployment)
	assertRESTServiceIngress(t, networkPolicy)
}

func flightConfigMap(t *testing.T, resources []*unstructured.Unstructured) *unstructured.Unstructured {
	t.Helper()
	for _, resource := range resources {
		if resource.GetKind() == kindConfigMap && resource.GetLabels()[labelAppName] == nameFlightService {
			return resource
		}
	}
	t.Fatal("rendered flight-service ConfigMap is missing")
	return nil
}

func tomlStringList(value any) []string {
	switch list := value.(type) {
	case []string:
		return list
	case []any:
		out := make([]string, 0, len(list))
		for _, item := range list {
			s, _ := item.(string)
			out = append(out, s)
		}
		return out
	}
	return nil
}

const tlsECDHERSAAES128GCM = "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256"

func TestSetConfigMapFlightTLSProfile(t *testing.T) {
	tests := []struct {
		name           string
		spec           configv1.TLSProfileSpec
		wantMin        configv1.TLSProtocolVersion
		wantCiphers    []string
		wantCiphersKey bool
	}{
		{
			name:    "Intermediate projects minimum and IANA cipher suites",
			spec:    *configv1.TLSProfiles[configv1.TLSProfileIntermediateType],
			wantMin: configv1.VersionTLS12,
			wantCiphers: []string{
				"TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
				tlsECDHERSAAES128GCM,
				"TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
				"TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
				"TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
				"TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
			},
			wantCiphersKey: true,
		},
		{
			name:    "Modern projects TLS 1.3 with runtime-managed ciphers",
			spec:    *configv1.TLSProfiles[configv1.TLSProfileModernType],
			wantMin: configv1.VersionTLS13,
		},
		{
			name: "Custom profile projects verbatim",
			spec: configv1.TLSProfileSpec{
				MinTLSVersion: configv1.VersionTLS12,
				Ciphers:       []string{tlsECDHERSAAES128GCM},
			},
			wantMin:        configv1.VersionTLS12,
			wantCiphers:    []string{tlsECDHERSAAES128GCM},
			wantCiphersKey: true,
		},
		{
			name: "Custom profile with no ciphers omits the cipher list",
			spec: configv1.TLSProfileSpec{
				MinTLSVersion: configv1.VersionTLS12,
			},
			wantMin:        configv1.VersionTLS12,
			wantCiphersKey: false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			resources := renderRESTBase(t)
			if err := setConfigMapFlightTLSProfile(context.Background(), resources, nameFlightService, tt.spec); err != nil {
				t.Fatalf("setConfigMapFlightTLSProfile() error = %v", err)
			}
			config := parsedConfigMapTOML(t, flightConfigMap(t, resources))
			tls, ok := config["tls"].(map[string]any)
			if !ok {
				t.Fatalf("flight config has no [tls] section: %#v", config)
			}
			if tls["min_version"] != string(tt.wantMin) {
				t.Errorf("tls.min_version = %v, want %s", tls["min_version"], tt.wantMin)
			}
			_, hasCiphers := tls["ciphers"]
			if hasCiphers != tt.wantCiphersKey {
				t.Errorf("tls.ciphers present = %t, want %t", hasCiphers, tt.wantCiphersKey)
			}
			if got := tomlStringList(tls["ciphers"]); len(got) != len(tt.wantCiphers) {
				t.Errorf("tls.ciphers = %v, want %v", got, tt.wantCiphers)
			} else {
				for i := range got {
					if got[i] != tt.wantCiphers[i] {
						t.Errorf("tls.ciphers[%d] = %q, want %q", i, got[i], tt.wantCiphers[i])
					}
				}
			}
			if tls["cert_file"] == nil || tls["key_file"] == nil {
				t.Errorf("projection dropped the TLS identity: %#v", tls)
			}
			// The REST ConfigMap must not grow a TLS policy.
			for _, resource := range resources {
				if resource.GetKind() != kindConfigMap {
					continue
				}
				config, _, err := parseConfigMapTOML(resource)
				if err != nil {
					t.Fatalf("parsing ConfigMap %q: %v", resource.GetName(), err)
				}
				if restTLS, ok := config["tls"].(map[string]any); ok && resource.GetLabels()[labelAppName] != nameFlightService {
					if restTLS["min_version"] != nil || restTLS["ciphers"] != nil {
						t.Errorf("ConfigMap %q unexpectedly received a TLS policy", resource.GetName())
					}
				}
			}
		})
	}
}

func TestSetConfigMapFlightTLSProfileOldProjectsForFailFastStartup(t *testing.T) {
	// Documented limitation: the Flight listener cannot serve the Old minimum
	// (rustls has no TLS 1.0/1.1), so the profile is projected verbatim and
	// the service fails fast at startup instead of silently upgrading the
	// minimum. The projected cipher list keeps the Old-only suites (3DES, CBC)
	// and drops the TLS 1.3 suites that cannot negotiate under TLS 1.0.
	resources := renderRESTBase(t)
	if err := setConfigMapFlightTLSProfile(context.Background(), resources, nameFlightService, *configv1.TLSProfiles[configv1.TLSProfileOldType]); err != nil {
		t.Fatalf("setConfigMapFlightTLSProfile() error = %v", err)
	}
	config := parsedConfigMapTOML(t, flightConfigMap(t, resources))
	tls := config["tls"].(map[string]any)
	if tls["min_version"] != string(configv1.VersionTLS10) {
		t.Errorf("tls.min_version = %v, want %s", tls["min_version"], configv1.VersionTLS10)
	}
	ciphers := tomlStringList(tls["ciphers"])
	if !containsString(ciphers, "TLS_RSA_WITH_3DES_EDE_CBC_SHA") {
		t.Errorf("tls.ciphers = %v, want the Old 3DES suite to survive projection", ciphers)
	}
	if !containsString(ciphers, tlsECDHERSAAES128GCM) {
		t.Errorf("tls.ciphers = %v, want the shared ECDHE suite to survive projection", ciphers)
	}
	for _, cipher := range ciphers {
		if strings.HasPrefix(cipher, "TLS_AES_") || strings.HasPrefix(cipher, "TLS_CHACHA20_POLY1305_SHA256") {
			t.Errorf("tls.ciphers = %v, want TLS 1.3-only suites dropped under a TLS 1.0 minimum", ciphers)
		}
	}
}

func TestSetConfigMapFlightTLSProfileRejectsUnusableCiphers(t *testing.T) {
	resources := renderRESTBase(t)
	spec := configv1.TLSProfileSpec{
		MinTLSVersion: configv1.VersionTLS12,
		Ciphers:       []string{"TLS_FAKE_CIPHER_WITH_NO_MAPPING"},
	}
	if err := setConfigMapFlightTLSProfile(context.Background(), resources, nameFlightService, spec); err == nil {
		t.Fatal("setConfigMapFlightTLSProfile() with an all-unusable cipher list = nil error, want rejection")
	}
}

func TestFlightTLSProfileChangesTriggerConfigHashRollout(t *testing.T) {
	resources := renderRESTBase(t)
	flightCM := flightConfigMap(t, resources)

	if err := setConfigMapFlightTLSProfile(context.Background(), resources, nameFlightService, *configv1.TLSProfiles[configv1.TLSProfileIntermediateType]); err != nil {
		t.Fatalf("projecting Intermediate: %v", err)
	}
	hashIntermediate := specHash(flightCM)

	if err := setConfigMapFlightTLSProfile(context.Background(), resources, nameFlightService, *configv1.TLSProfiles[configv1.TLSProfileModernType]); err != nil {
		t.Fatalf("projecting Modern: %v", err)
	}
	hashModern := specHash(flightCM)

	if hashIntermediate == "" || hashModern == "" || hashIntermediate == hashModern {
		t.Errorf("config hash did not change with the TLS profile: intermediate=%q modern=%q", hashIntermediate, hashModern)
	}
}

func renderRESTBase(t *testing.T) []*unstructured.Unstructured {
	t.Helper()
	manifestsPath := filepath.Join("..", "..", "..", "config")
	resources, err := renderKustomization(manifestsPath, filepath.Join(manifestsPath, "base"), nil, nil)
	if err != nil {
		t.Fatalf("rendering base manifests: %v", err)
	}
	return resources
}

func findRESTServiceResources(t *testing.T, resources []*unstructured.Unstructured) (*unstructured.Unstructured, *unstructured.Unstructured, *unstructured.Unstructured) {
	t.Helper()
	var configMap, deployment, networkPolicy *unstructured.Unstructured
	for _, resource := range resources {
		switch resource.GetKind() {
		case kindConfigMap:
			if strings.HasSuffix(resource.GetName(), "rest-service-config") {
				configMap = resource
			}
		case kindDeployment:
			if strings.HasSuffix(resource.GetName(), nameRestService) {
				deployment = resource
			}
		case kindNetworkPolicy:
			if strings.HasSuffix(resource.GetName(), nameRestService) {
				networkPolicy = resource
			}
		}
	}
	if configMap == nil || deployment == nil || networkPolicy == nil {
		t.Fatal("rendered REST ConfigMap, Deployment, or NetworkPolicy is missing")
	}
	return configMap, deployment, networkPolicy
}

func assertRESTServiceConfig(t *testing.T, configMap *unstructured.Unstructured) {
	t.Helper()
	config := parsedConfigMapTOML(t, configMap)
	server := config["server"].(map[string]any)
	if server["address"] != "127.0.0.1" || server["port"] != int64(8080) {
		t.Errorf("REST server config = %#v, want loopback:8080", server)
	}
	metrics := config["metrics"].(map[string]any)
	if metrics["address"] != "0.0.0.0" || metrics["port"] != int64(9090) {
		t.Errorf("metrics config = %#v, want 0.0.0.0:9090", metrics)
	}
}

func assertRESTServiceProbes(t *testing.T, deployment *unstructured.Unstructured) {
	t.Helper()
	containers, _, _ := unstructured.NestedSlice(deployment.Object, "spec", testTemplateKey, "spec", testContainersKey)
	var restContainer map[string]any
	for _, raw := range containers {
		container, ok := raw.(map[string]any)
		if ok && container[resourceNameField] == nameRestServiceContainer {
			restContainer = container
		}
	}
	if restContainer == nil {
		t.Fatal("rendered REST deployment has no rest-server container")
	}
	for _, probeName := range []string{"readinessProbe", "livenessProbe"} {
		probe, ok := restContainer[probeName].(map[string]any)
		if !ok {
			t.Errorf("%s is missing", probeName)
			continue
		}
		httpGet, _ := probe["httpGet"].(map[string]any)
		if httpGet["scheme"] != "HTTPS" || httpGet["path"] != "/health" || httpGet["port"] != int64(8443) {
			t.Errorf("%s httpGet = %#v, want HTTPS /health on port 8443", probeName, httpGet)
		}
		wantDelay, wantPeriod := int64(3), int64(10)
		if probeName == "livenessProbe" {
			wantDelay, wantPeriod = 5, 15
		}
		if probe["initialDelaySeconds"] != wantDelay || probe["periodSeconds"] != wantPeriod {
			t.Errorf("%s timing = %v/%v, want %d/%d", probeName, probe["initialDelaySeconds"], probe["periodSeconds"], wantDelay, wantPeriod)
		}
	}
}

func assertRESTServiceIngress(t *testing.T, networkPolicy *unstructured.Unstructured) {
	t.Helper()
	ingress, found, err := unstructured.NestedSlice(networkPolicy.Object, "spec", "ingress")
	if err != nil || !found || len(ingress) == 0 {
		t.Fatalf("ingress rules = %v, found=%t err=%v, want non-empty ingress", ingress, found, err)
	}
	ports := map[int64]bool{}
	for _, rawRule := range ingress {
		rule, ok := rawRule.(map[string]any)
		if !ok {
			t.Fatalf("ingress rule has unexpected type %T", rawRule)
		}
		rawPorts, ok := rule["ports"].([]any)
		if !ok || len(rawPorts) != 1 {
			t.Fatalf("ingress rule ports = %#v, want one port", rule["ports"])
		}
		port := rawPorts[0].(map[string]any)
		if port["protocol"] != "TCP" {
			t.Errorf("ingress protocol = %v, want TCP", port["protocol"])
		}
		portNum, ok := port["port"].(int64)
		if !ok {
			t.Fatalf("ingress port has unexpected type %T", port["port"])
		}
		ports[portNum] = true
		if portNum == 8443 {
			// The TLS proxy port must be source-restricted: a missing or empty
			// from list means unrestricted ingress (CWE-284).
			from, ok := rule["from"].([]any)
			if !ok || len(from) == 0 {
				t.Errorf("ingress on 8443 is unrestricted: %#v", rule)
				continue
			}
			for _, rawPeer := range from {
				peer, ok := rawPeer.(map[string]any)
				if !ok {
					t.Fatalf("ingress peer has unexpected type %T", rawPeer)
				}
				ns, ok := peer["namespaceSelector"].(map[string]any)
				if !ok {
					t.Fatalf("ingress peer on 8443 has no namespaceSelector: %#v", peer)
				}
				matchLabels, _ := ns["matchLabels"].(map[string]any)
				if len(matchLabels) == 0 {
					t.Errorf("ingress peer on 8443 has an empty namespaceSelector: %#v", peer)
				}
			}
		}
	}
	if !ports[8443] || !ports[9090] {
		t.Errorf("ingress ports = %v, want 8443 and 9090", ports)
	}
	if len(ports) != 2 {
		t.Errorf("ingress ports = %v, want only 8443 and 9090", ports)
	}
}

func kubeRbacProxyDeployment(args ...any) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindDeployment,
		testSpecKey: map[string]any{testTemplateKey: map[string]any{testSpecKey: map[string]any{
			testContainersKey: []any{map[string]any{resourceNameField: nameKubeRbacProxy, "args": args}},
		}}},
	}}
}

func kubeRbacProxyArgs(t *testing.T, deployment *unstructured.Unstructured) []string {
	t.Helper()
	containers, _, err := unstructured.NestedSlice(deployment.Object, "spec", testTemplateKey, "spec", testContainersKey)
	if err != nil {
		t.Fatal(err)
	}
	container := containers[0].(map[string]any)
	args := container["args"].([]any)
	result := make([]string, len(args))
	for i, arg := range args {
		result[i] = arg.(string)
	}
	return result
}

func setKubeRbacProxyArgs(t *testing.T, deployment *unstructured.Unstructured, args []string) {
	t.Helper()
	containers, _, err := unstructured.NestedSlice(deployment.Object, "spec", testTemplateKey, "spec", testContainersKey)
	if err != nil {
		t.Fatal(err)
	}
	container := containers[0].(map[string]any)
	values := make([]any, len(args))
	for i := range args {
		values[i] = args[i]
	}
	container["args"] = values
	containers[0] = container
	if err := unstructured.SetNestedSlice(deployment.Object, containers, "spec", testTemplateKey, "spec", testContainersKey); err != nil {
		t.Fatal(err)
	}
}

func renderedRESTResources(t *testing.T) ([]*unstructured.Unstructured, *unstructured.Unstructured) {
	t.Helper()
	manifestsPath := filepath.Join("..", "..", "..", "config")
	resources, err := renderKustomization(manifestsPath, filepath.Join(manifestsPath, "base"), nil, nil)
	if err != nil {
		t.Fatalf("rendering base manifests: %v", err)
	}
	for _, resource := range resources {
		if resource.GetKind() != kindDeployment || !strings.HasSuffix(resource.GetName(), nameRestService) {
			continue
		}
		return resources, resource
	}
	t.Fatal("rendered REST deployment not found")
	return nil, nil
}

func containsString(values []string, want string) bool {
	return slices.Contains(values, want)
}

func containsArgPrefix(values []string, prefix string) bool {
	for _, value := range values {
		if strings.HasPrefix(value, prefix) {
			return true
		}
	}
	return false
}

func countArgPrefix(values []string, prefix string) int {
	count := 0
	for _, value := range values {
		if strings.HasPrefix(value, prefix) {
			count++
		}
	}
	return count
}

func TestRenderKustomizationImageParams(t *testing.T) {
	manifestsPath := filepath.Join("..", "..", "..", "config")
	paramsPath := filepath.Join(manifestsPath, testBaseManifest, "params.env")
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
		{name: testBaseManifest, path: filepath.Join(manifestsPath, testBaseManifest)},
		{name: "openshift overlay", path: filepath.Join(manifestsPath, "overlays", "openshift")},
	}

	for _, tt := range paths {
		t.Run(tt.name, func(t *testing.T) {
			resources, err := renderKustomization(manifestsPath, tt.path, nil, imageParams)
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
		containers, found, err := unstructured.NestedSlice(obj.Object, "spec", testTemplateKey, "spec", testContainersKey)
		if err != nil || !found {
			continue
		}
		for _, raw := range containers {
			container, ok := raw.(map[string]any)
			if !ok || container[resourceNameField] != containerName {
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
			resourceNameField: "default-dcs-flight-config",
		},
		testDataKey: map[string]any{
			testConfigTOMLKey: "[connectors.uri]\nenabled = false\n",
		},
	}}
	deployment := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindDeployment,
		testMetadataKey: map[string]any{
			resourceNameField: "default-dcs-flight",
		},
		testSpecKey: map[string]any{
			testTemplateKey: map[string]any{
				testSpecKey: map[string]any{
					testContainersKey: []any{
						map[string]any{
							resourceNameField: nameFlightServiceContainer,
							"image":           "localhost/dch-flight:test",
						},
					},
					"volumes": []any{
						map[string]any{
							resourceNameField: "config",
							"configMap":       map[string]any{resourceNameField: "default-dcs-flight-config"},
						},
						map[string]any{
							resourceNameField: "tls",
							"secret":          map[string]any{"secretName": "default-dcs-flight-tls"},
						},
						map[string]any{
							resourceNameField: "db-secret",
							"secret":          map[string]any{"secretName": nameDatabaseConfig},
						},
						map[string]any{
							resourceNameField: "tmp",
							"emptyDir":        map[string]any{},
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
		deployment.Object, testSpecKey, testTemplateKey, testMetadataKey, "annotations",
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
		deployment2.Object, testSpecKey, testTemplateKey, testMetadataKey, "annotations",
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
			resourceNameField: testFlightServiceCA,
		},
		testDataKey: map[string]any{},
	}}
	deployment := &unstructured.Unstructured{Object: map[string]any{
		testKindKey: kindDeployment,
		testMetadataKey: map[string]any{
			resourceNameField: nameRestService,
		},
		testSpecKey: map[string]any{
			testTemplateKey: map[string]any{
				testSpecKey: map[string]any{
					testContainersKey: []any{
						map[string]any{resourceNameField: nameRestServiceContainer, "image": "test:latest"},
					},
					"volumes": []any{
						map[string]any{
							resourceNameField: "flight-ca",
							"configMap":       map[string]any{resourceNameField: testFlightServiceCA},
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
		deployment.Object, testSpecKey, testTemplateKey, testMetadataKey, "annotations",
	)
	if !found || annotations[annotationConfigHash] == "" {
		t.Fatal("expected hash annotation from live-fetched CA ConfigMap")
	}
}
