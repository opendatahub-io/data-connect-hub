package manifests_test

import (
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"sigs.k8s.io/kustomize/api/krusty"
	"sigs.k8s.io/kustomize/kyaml/filesys"
	"sigs.k8s.io/kustomize/kyaml/kio"
	"sigs.k8s.io/yaml"
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

// TestServiceMonitorOnlyInOpenShiftOverlay guards the split between the manifest
// roots the controller renders: base/ must stay applicable to any cluster, while
// the ServiceMonitor ships only in overlays/openshift.
func TestServiceMonitorOnlyInOpenShiftOverlay(t *testing.T) {
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not locate manifest test")
	}
	configRoot := filepath.Clean(filepath.Join(filepath.Dir(filename), "..", "..", "..", "config"))

	baseNames := renderKinds(t, filepath.Join(configRoot, "base"), "ServiceMonitor")
	if len(baseNames) > 0 {
		t.Fatalf("base renders ServiceMonitor(s) %v, want none", baseNames)
	}

	overlayNames := renderKinds(t, filepath.Join(configRoot, "overlays", "openshift"), "ServiceMonitor")
	if len(overlayNames) != 1 || overlayNames[0] != "dch-servicemonitor" {
		t.Fatalf("openshift overlay ServiceMonitors = %v, want [dch-servicemonitor]", overlayNames)
	}

	deployments := renderKinds(t, filepath.Join(configRoot, "overlays", "openshift"), "Deployment")
	if len(deployments) != 2 {
		t.Fatalf("openshift overlay Deployments = %v, want the two inherited from base", deployments)
	}
}

func TestRestServiceExposesMetricsPort(t *testing.T) {
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not locate manifest test")
	}
	configRoot := filepath.Clean(filepath.Join(filepath.Dir(filename), "..", "..", "..", "config"))

	kustomizer := krusty.MakeKustomizer(krusty.MakeDefaultOptions())
	resMap, err := kustomizer.Run(filesys.MakeFsOnDisk(), filepath.Join(configRoot, "base"))
	if err != nil {
		t.Fatalf("rendering base manifests: %v", err)
	}
	rendered, err := resMap.AsYaml()
	if err != nil {
		t.Fatalf("serializing base manifests: %v", err)
	}
	nodes, err := (&kio.ByteReader{Reader: strings.NewReader(string(rendered))}).Read()
	if err != nil {
		t.Fatalf("parsing base manifests: %v", err)
	}

	servicePortFound := false
	containerPortFound := false
	const metricsPortName = "metrics"
	for _, node := range nodes {
		var object struct {
			Kind     string `yaml:"kind"`
			Metadata struct {
				Name string `yaml:"name"`
			} `yaml:"metadata"`
			Spec struct {
				Ports []struct {
					Name       string `yaml:"name"`
					Port       int    `yaml:"port"`
					TargetPort string `yaml:"targetPort"`
				} `yaml:"ports"`
				Template struct {
					Spec struct {
						Containers []struct {
							Name  string `yaml:"name"`
							Ports []struct {
								Name          string `yaml:"name"`
								ContainerPort int    `yaml:"containerPort"`
							} `yaml:"ports"`
						} `yaml:"containers"`
					} `yaml:"spec"`
				} `yaml:"template"`
			} `yaml:"spec"`
		}
		if err := yaml.Unmarshal([]byte(node.MustString()), &object); err != nil {
			t.Fatalf("parsing rendered resource: %v", err)
		}
		if object.Metadata.Name != "dch-rest-service" {
			continue
		}

		switch object.Kind {
		case "Service":
			for _, port := range object.Spec.Ports {
				if port.Name == metricsPortName && port.Port == 9090 && port.TargetPort == metricsPortName {
					servicePortFound = true
				}
			}
		case "Deployment":
			for _, container := range object.Spec.Template.Spec.Containers {
				if container.Name != "rest-service" {
					continue
				}
				for _, port := range container.Ports {
					if port.Name == metricsPortName && port.ContainerPort == 9090 {
						containerPortFound = true
					}
				}
			}
		}
	}

	if !servicePortFound {
		t.Error("rendered REST Service does not expose metrics port 9090")
	}
	if !containerPortFound {
		t.Error("rendered REST Deployment does not declare metrics container port 9090")
	}
}

// renderKinds renders the kustomization at path and returns the names of every
// rendered resource of the given kind.
func renderKinds(t *testing.T, path, kind string) []string {
	t.Helper()

	kustomizer := krusty.MakeKustomizer(krusty.MakeDefaultOptions())
	resMap, err := kustomizer.Run(filesys.MakeFsOnDisk(), path)
	if err != nil {
		t.Fatalf("rendering %s: %v", path, err)
	}

	var names []string
	for _, res := range resMap.Resources() {
		if res.GetKind() == kind {
			names = append(names, res.GetName())
		}
	}
	return names
}
