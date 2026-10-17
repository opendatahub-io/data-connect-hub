package manifests_test

import (
	"bytes"
	"io"
	"maps"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"helm.sh/helm/v3/pkg/chart/loader"
	"helm.sh/helm/v3/pkg/chartutil"
	"helm.sh/helm/v3/pkg/engine"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	k8syaml "k8s.io/apimachinery/pkg/util/yaml"
	"sigs.k8s.io/kustomize/api/krusty"
	"sigs.k8s.io/kustomize/kyaml/filesys"
	"sigs.k8s.io/kustomize/kyaml/kio"
	"sigs.k8s.io/yaml"
)

const (
	nameKey           = "name"
	cpuKey            = "cpu"
	memoryKey         = "memory"
	requestsKey       = "requests"
	resourcesKey      = "resources"
	limitsKey         = "limits"
	audiencesKey      = "tokenReviewAudiences"
	defaultTenant     = "opendatahub"
	defaultListenAddr = "0.0.0.0"
	memory256Mi       = "256Mi"
)

func TestXKSMetricsNetworkPolicyUsesHTTPPort(t *testing.T) {
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not locate manifest test")
	}

	controllerRoot := filepath.Clean(filepath.Join(filepath.Dir(filename), "..", ".."))
	kustomizer := krusty.MakeKustomizer(krusty.MakeDefaultOptions())
	resMap, err := kustomizer.Run(filesys.MakeFsOnDisk(), filepath.Join(controllerRoot, "config", "overlays", "xks"))
	if err != nil {
		t.Fatalf("rendering XKS overlay: %v", err)
	}

	rendered, err := resMap.AsYaml()
	if err != nil {
		t.Fatalf("serializing XKS overlay: %v", err)
	}
	nodes, err := (&kio.ByteReader{Reader: strings.NewReader(string(rendered))}).Read()
	if err != nil {
		t.Fatalf("parsing XKS overlay: %v", err)
	}

	for _, node := range nodes {
		var object struct {
			Kind     string `yaml:"kind"`
			Metadata struct {
				Name string `yaml:"name"`
			} `yaml:"metadata"`
			Spec struct {
				Ingress []struct {
					Ports []struct {
						Port int `yaml:"port"`
					} `yaml:"ports"`
				} `yaml:"ingress"`
			} `yaml:"spec"`
		}
		if err := yaml.Unmarshal([]byte(node.MustString()), &object); err != nil {
			t.Fatalf("parsing rendered resource: %v", err)
		}
		if object.Kind != "NetworkPolicy" || !strings.HasSuffix(object.Metadata.Name, "allow-metrics-traffic") {
			continue
		}
		if len(object.Spec.Ingress) == 0 || len(object.Spec.Ingress[0].Ports) == 0 {
			t.Fatal("rendered metrics NetworkPolicy has no ingress port")
		}
		if object.Spec.Ingress[0].Ports[0].Port != 8080 {
			t.Fatalf("rendered XKS metrics port = %d, want 8080", object.Spec.Ingress[0].Ports[0].Port)
		}
		return
	}

	t.Fatal("rendered XKS overlay did not contain the metrics NetworkPolicy")
}

// baseChartValues returns the Helm values the controller derives from a
// DataConnectService CR, shared by the chart rendering tests below.
func baseChartValues() map[string]any {
	return map[string]any{
		"namePrefix": "dch-",
		"images": map[string]any{
			"restService":   "test:rest",
			"flightService": "test:flight",
			"kubeRbacProxy": "test:proxy",
		},
		"restService": map[string]any{
			"replicas": 1,
			resourcesKey: map[string]any{
				requestsKey: map[string]any{cpuKey: "100m", memoryKey: memory256Mi},
				limitsKey:   map[string]any{cpuKey: "1", memoryKey: "512Mi"},
			},
			"env":          []any{},
			"envFrom":      []any{},
			"volumes":      []any{},
			"volumeMounts": []any{},
			"config": map[string]any{
				audiencesKey: []any{},
			},
		},
		"kubeRbacProxy": map[string]any{
			resourcesKey: map[string]any{
				requestsKey: map[string]any{cpuKey: "50m", memoryKey: "64Mi"},
				limitsKey:   map[string]any{cpuKey: "500m", memoryKey: memory256Mi},
			},
			audiencesKey: []any{},
		},
		"flightService": map[string]any{
			"name":     "default-flight",
			"replicas": 1,
			resourcesKey: map[string]any{
				requestsKey: map[string]any{cpuKey: "100m", memoryKey: memory256Mi},
				limitsKey:   map[string]any{cpuKey: "1", memoryKey: "512Mi"},
			},
			"env":          []any{},
			"envFrom":      []any{},
			"volumes":      []any{},
			"volumeMounts": []any{},
			"config": map[string]any{
				audiencesKey: []any{},
			},
		},
		"gateway": map[string]any{
			"enabled": true,
			"parentRef": map[string]any{
				nameKey:     "odh-gateway",
				"namespace": defaultTenant,
			},
		},
		"trace": map[string]any{"endpoint": "", "insecure": "", "certificate": ""},
	}
}

func monitoringValues(enabled bool) map[string]any {
	return map[string]any{"enabled": enabled, "scrapeInterval": "30s"}
}

func TestServiceMonitorOnlyWhenMonitoringEnabled(t *testing.T) {
	chartPath := operandChartPath(t)

	withoutMonitoring := maps.Clone(baseChartValues())
	withoutMonitoring["monitoring"] = monitoringValues(false)

	baseResources := renderChart(t, chartPath, withoutMonitoring, "test-ns")
	baseMonitors := filterByKind(baseResources, "ServiceMonitor")
	if len(baseMonitors) > 0 {
		t.Fatalf("chart without monitoring renders ServiceMonitor(s), want none")
	}

	withMonitoring := maps.Clone(baseChartValues())
	withMonitoring["monitoring"] = monitoringValues(true)

	overlayResources := renderChart(t, chartPath, withMonitoring, "test-ns")
	overlayMonitors := filterByKind(overlayResources, "ServiceMonitor")
	if len(overlayMonitors) != 1 {
		t.Fatalf("chart with monitoring renders %d ServiceMonitor(s), want 1", len(overlayMonitors))
	}

	deployments := filterByKind(overlayResources, "Deployment")
	if len(deployments) != 2 {
		t.Fatalf("chart with monitoring renders %d Deployments, want 2", len(deployments))
	}
}

func TestRestServiceExposesMetricsPort(t *testing.T) {
	chartPath := operandChartPath(t)

	values := baseChartValues()
	values["monitoring"] = monitoringValues(false)

	resources := renderChart(t, chartPath, values, "test-ns")

	servicePortFound := false
	containerPortFound := false
	for _, obj := range resources {
		if obj.GetName() != "dch-rest-service" {
			continue
		}
		switch obj.GetKind() {
		case "Service":
			ports, _, _ := unstructured.NestedSlice(obj.Object, "spec", "ports")
			for _, p := range ports {
				port, _ := p.(map[string]any)
				if port[nameKey] == "metrics" {
					servicePortFound = true
				}
			}
		case "Deployment":
			containers, _, _ := unstructured.NestedSlice(obj.Object, "spec", "template", "spec", "containers")
			for _, c := range containers {
				container, _ := c.(map[string]any)
				if container[nameKey] != "rest-server" {
					continue
				}
				ports, _ := container["ports"].([]any)
				for _, p := range ports {
					port, _ := p.(map[string]any)
					if port[nameKey] == "metrics" {
						containerPortFound = true
					}
				}
			}
		}
	}

	if !servicePortFound {
		t.Error("rendered REST Service does not expose metrics port")
	}
	if !containerPortFound {
		t.Error("rendered REST Deployment does not declare metrics container port")
	}
}

func operandChartPath(t *testing.T) string {
	t.Helper()
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not locate manifest test")
	}
	chartPath := filepath.Join(filepath.Dir(filename), "..", "..", "..", "manifests", "charts", "data-connect-hub")
	return filepath.Clean(chartPath)
}

func renderChart(t *testing.T, chartPath string, values map[string]any, namespace string) []*unstructured.Unstructured {
	t.Helper()
	c, err := loader.Load(chartPath)
	if err != nil {
		t.Fatalf("loading chart: %v", err)
	}
	releaseOpts := chartutil.ReleaseOptions{
		Name:      "dch",
		Namespace: namespace,
		IsInstall: true,
	}
	valuesToRender, err := chartutil.ToRenderValues(c, values, releaseOpts, nil)
	if err != nil {
		t.Fatalf("preparing values: %v", err)
	}
	rendered, err := engine.Render(c, valuesToRender)
	if err != nil {
		t.Fatalf("rendering: %v", err)
	}

	var objects []*unstructured.Unstructured
	for name, content := range rendered {
		if strings.HasSuffix(name, "_helpers.tpl") || strings.HasSuffix(name, "NOTES.txt") {
			continue
		}
		trimmed := strings.TrimSpace(content)
		if trimmed == "" {
			continue
		}
		decoder := k8syaml.NewYAMLToJSONDecoder(bytes.NewBufferString(trimmed))
		for {
			obj := &unstructured.Unstructured{}
			if err := decoder.Decode(&obj.Object); err != nil {
				if err == io.EOF {
					break
				}
				t.Fatalf("decoding %s: %v", name, err)
			}
			if obj.Object == nil {
				continue
			}
			objects = append(objects, obj)
		}
	}
	return objects
}

func filterByKind(resources []*unstructured.Unstructured, kind string) []*unstructured.Unstructured {
	var result []*unstructured.Unstructured
	for _, r := range resources {
		if r.GetKind() == kind {
			result = append(result, r)
		}
	}
	return result
}
