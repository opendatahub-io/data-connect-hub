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
