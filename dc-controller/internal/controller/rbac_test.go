package controller

import (
	"os"
	"path/filepath"
	"testing"

	rbacv1 "k8s.io/api/rbac/v1"
	"sigs.k8s.io/yaml"
)

func TestControllerCanManageInitialConnectionTypes(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "config", "rbac", "role.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	var role rbacv1.ClusterRole
	if err := yaml.Unmarshal(data, &role); err != nil {
		t.Fatal(err)
	}
	requiredVerbs := map[string]bool{"create": false, "delete": false}
	for _, rule := range role.Rules {
		if len(rule.ResourceNames) != 0 {
			continue
		}
		for _, group := range rule.APIGroups {
			for _, resource := range rule.Resources {
				for _, verb := range rule.Verbs {
					if group == "dataconnecthub.opendatahub.io" && resource == "initdataconnectiontypes" {
						if _, ok := requiredVerbs[verb]; ok {
							requiredVerbs[verb] = true
						}
					}
				}
			}
		}
	}
	for verb, granted := range requiredVerbs {
		if !granted {
			t.Errorf("controller RBAC must allow %s on initdataconnectiontypes", verb)
		}
	}
}
