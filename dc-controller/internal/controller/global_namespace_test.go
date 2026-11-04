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
	"errors"
	"slices"
	"testing"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/event"
)

const (
	testGroupAuthenticated = "system:authenticated"
	testGroupAdmins        = "rhods-admins"
	testGroupPlatformAdmin = "platform-admins"
	testServiceNamespace   = "dch"
)

func TestLoadGlobalNamespaceConfig(t *testing.T) {
	available := func(ok bool) func() (bool, error) {
		return func() (bool, error) { return ok, nil }
	}
	discoveryErr := func() (bool, error) { return false, errors.New("boom") }

	tests := []struct {
		name        string
		enable      string
		label       string
		probe       func() (bool, error)
		wantEnabled bool
		wantLabel   string
		wantErr     bool
	}{
		{name: "unset enables when Auth CRD exists", probe: available(true), wantEnabled: true, wantLabel: DefaultGlobalNamespaceLabel},
		{name: "unset disables when Auth CRD is missing", probe: available(false), wantLabel: DefaultGlobalNamespaceLabel},
		{name: "false disables without probing", enable: valueFalse, probe: discoveryErr, wantLabel: DefaultGlobalNamespaceLabel},
		{name: "true requires Auth CRD", enable: valueTrue, probe: available(false), wantErr: true},
		{name: "true with Auth CRD", enable: valueTrue, probe: available(true), wantEnabled: true, wantLabel: DefaultGlobalNamespaceLabel},
		{name: "invalid flag", enable: "maybe", probe: available(true), wantErr: true},
		{name: "probe error", probe: discoveryErr, wantErr: true},
		{name: "custom label", label: " opendatahub.io/global-project ", probe: available(true), wantEnabled: true, wantLabel: "opendatahub.io/global-project"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Setenv(EnvEnableGlobalNamespace, tt.enable)
			t.Setenv(EnvGlobalNamespaceLabel, tt.label)
			cfg, err := LoadGlobalNamespaceConfig(tt.probe)
			if tt.wantErr {
				if err == nil {
					t.Fatalf("expected an error, got config %+v", cfg)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if cfg.Enabled != tt.wantEnabled || cfg.LabelKey != tt.wantLabel {
				t.Errorf("got %+v, want enabled=%v label=%q", cfg, tt.wantEnabled, tt.wantLabel)
			}
		})
	}
}

func TestUniqueSorted(t *testing.T) {
	got := uniqueSorted([]string{"b", "a", " ", "b"}, []string{"a", "c", ""})
	want := []string{"a", "b", "c"}
	if !slices.Equal(got, want) {
		t.Errorf("uniqueSorted() = %v, want %v", got, want)
	}
}

func TestDesiredGlobalRoleBindings(t *testing.T) {
	rest := []serviceAccountRef{{Namespace: testServiceNamespace, Name: serviceAccountRestService}}
	flight := []serviceAccountRef{{Namespace: testServiceNamespace, Name: flightServiceAccountName("default")}}

	byName := func(rbs []*rbacv1.RoleBinding) map[string]*rbacv1.RoleBinding {
		m := map[string]*rbacv1.RoleBinding{}
		for _, rb := range rbs {
			m[rb.Name] = rb
		}
		return m
	}
	groupNames := func(rb *rbacv1.RoleBinding) []string {
		names := make([]string, 0, len(rb.Subjects))
		for _, s := range rb.Subjects {
			if s.Kind != rbacv1.GroupKind || s.APIGroup != rbacv1.GroupName {
				t.Errorf("%s: unexpected subject %+v", rb.Name, s)
			}
			names = append(names, s.Name)
		}
		return names
	}

	t.Run("maps Auth groups and service accounts", func(t *testing.T) {
		rbs := byName(desiredGlobalRoleBindings("global",
			[]string{testGroupAuthenticated, testGroupAdmins}, []string{testGroupAdmins}, rest, flight))
		if len(rbs) != len(managedGlobalRoleBindings) {
			t.Fatalf("got %d RoleBindings, want %d", len(rbs), len(managedGlobalRoleBindings))
		}

		read := rbs[roleBindingGlobalRead]
		if read.RoleRef.Name != clusterRoleRead {
			t.Errorf("read roleRef = %s", read.RoleRef.Name)
		}
		if got := groupNames(read); !slices.Equal(got, []string{testGroupAdmins, testGroupAuthenticated}) {
			t.Errorf("read subjects = %v", got)
		}

		write := rbs[roleBindingGlobalReadWrite]
		if write.RoleRef.Name != clusterRoleReadWrite {
			t.Errorf("read-write roleRef = %s", write.RoleRef.Name)
		}
		if got := groupNames(write); !slices.Equal(got, []string{testGroupAdmins}) {
			t.Errorf("read-write subjects = %v", got)
		}

		restRB := rbs[roleBindingGlobalRestSecret]
		if restRB.RoleRef.Name != clusterRoleRestSecretAccess ||
			len(restRB.Subjects) != 1 || restRB.Subjects[0].Name != "dch-rest-service-sa" || restRB.Subjects[0].Namespace != testServiceNamespace {
			t.Errorf("rest secret binding = %+v", restRB)
		}
		flightRB := rbs[roleBindingGlobalFlightSecret]
		if flightRB.RoleRef.Name != clusterRoleFlightSecretRead ||
			len(flightRB.Subjects) != 1 || flightRB.Subjects[0].Name != "dch-default-flight-sa" {
			t.Errorf("flight secret binding = %+v", flightRB)
		}

		for _, rb := range rbs {
			if rb.Namespace != "global" || rb.Labels[labelGlobalNamespace] != valueTrue {
				t.Errorf("%s: namespace=%s labels=%v", rb.Name, rb.Namespace, rb.Labels)
			}
		}
	})

	t.Run("allowed groups include admin groups", func(t *testing.T) {
		rbs := byName(desiredGlobalRoleBindings("global", nil, []string{testGroupAdmins}, rest, flight))
		if got := groupNames(rbs[roleBindingGlobalRead]); !slices.Equal(got, []string{testGroupAdmins}) {
			t.Errorf("read subjects = %v", got)
		}
	})

	t.Run("bindings without subjects are omitted", func(t *testing.T) {
		rbs := byName(desiredGlobalRoleBindings("global", []string{testGroupAuthenticated}, nil, nil, nil))
		if _, ok := rbs[roleBindingGlobalReadWrite]; ok {
			t.Error("read-write binding rendered without admin groups")
		}
		if _, ok := rbs[roleBindingGlobalRestSecret]; ok {
			t.Error("rest secret binding rendered without service accounts")
		}
		if _, ok := rbs[roleBindingGlobalRead]; !ok {
			t.Error("read binding missing")
		}
	})
}

func TestFlightServiceAccountName(t *testing.T) {
	// renderFlightService rewrites "flight-service" in the base
	// "dch-flight-service-sa" name to "<cr>-flight".
	if got := flightServiceAccountName("default-dataconnectservice"); got != "dch-default-dataconnectservice-flight-sa" {
		t.Errorf("flightServiceAccountName() = %q", got)
	}
}

func TestLabelKeyPredicate(t *testing.T) {
	const key = "example.com/global"
	ns := func(labels map[string]string, deleting bool) *corev1.Namespace {
		n := &corev1.Namespace{ObjectMeta: metav1.ObjectMeta{Name: "ns", Labels: labels}}
		if deleting {
			now := metav1.Now()
			n.DeletionTimestamp = &now
		}
		return n
	}
	labeled := map[string]string{key: "x"}
	p := labelKeyPredicate[*corev1.Namespace](key)

	if !p.Create(event.TypedCreateEvent[*corev1.Namespace]{Object: ns(labeled, false)}) {
		t.Error("create of a labeled namespace should pass")
	}
	if p.Create(event.TypedCreateEvent[*corev1.Namespace]{Object: ns(nil, false)}) {
		t.Error("create of an unlabeled namespace should not pass")
	}
	if !p.Delete(event.TypedDeleteEvent[*corev1.Namespace]{Object: ns(labeled, false)}) {
		t.Error("delete of a labeled namespace should pass")
	}

	updates := []struct {
		name     string
		old, new *corev1.Namespace
		want     bool
	}{
		{"label added", ns(nil, false), ns(labeled, false), true},
		{"label removed", ns(labeled, false), ns(nil, false), true},
		{"label value changed", ns(labeled, false), ns(map[string]string{key: "y"}, false), true},
		{"deletion started", ns(labeled, false), ns(labeled, true), true},
		{"unrelated change on labeled namespace", ns(labeled, false), ns(map[string]string{key: "x", "other": "1"}, false), false},
		{"unlabeled namespace", ns(nil, false), ns(map[string]string{"other": "1"}, false), false},
	}
	for _, u := range updates {
		if got := p.Update(event.TypedUpdateEvent[*corev1.Namespace]{ObjectOld: u.old, ObjectNew: u.new}); got != u.want {
			t.Errorf("%s: got %v, want %v", u.name, got, u.want)
		}
	}
}
