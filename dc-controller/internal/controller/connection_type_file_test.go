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
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"sigs.k8s.io/yaml"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// OCI access type enum values and labels used by the test fixtures and
// expectations.
const (
	testAccessTypePush      = "Push"
	testAccessTypePull      = "Pull"
	testAccessTypePushLabel = "Push secret"
	testAccessTypePullLabel = "Pull secret"
)

// loadBuiltinConnectionTypeFiles parses every built-in connection type file
// under config/connection-types, keyed by file name.
func loadBuiltinConnectionTypeFiles(t *testing.T) map[string]connectionTypeFile {
	t.Helper()
	typesDir := filepath.Join("..", "..", "..", "config", "connection-types")
	entries, err := os.ReadDir(typesDir)
	if err != nil {
		t.Fatalf("read %s: %v", typesDir, err)
	}

	files := make(map[string]connectionTypeFile)
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".yaml" {
			continue
		}
		data, err := os.ReadFile(filepath.Join(typesDir, entry.Name()))
		if err != nil {
			t.Fatalf("read %s: %v", entry.Name(), err)
		}
		var file connectionTypeFile
		if err := yaml.Unmarshal(data, &file); err != nil {
			t.Fatalf("parse %s: %v", entry.Name(), err)
		}
		files[entry.Name()] = file
	}
	if len(files) == 0 {
		t.Fatalf("no connection type files found under %s", typesDir)
	}
	return files
}

// TestBuiltinConnectionTypesCoverAliases ensures every ODH Dashboard
// ConfigMap skipped by the ConfigMap watcher has exactly one built-in
// connection type for the Secret watcher to resolve to.
func TestBuiltinConnectionTypesCoverAliases(t *testing.T) {
	providers := make(map[string]int)
	for _, file := range loadBuiltinConnectionTypeFiles(t) {
		providers[file.Provider]++
	}
	for cmName, provider := range builtinProviderAliases {
		if n := providers[provider]; n != 1 {
			t.Errorf("alias %s: found %d built-in connection types with provider %q, want 1", cmName, n, provider)
		}
	}
}

func TestConnectionTypeFileEnumField(t *testing.T) {
	data := []byte(`
name: OCI
provider: oci
credentials_fields:
  - name: ACCESS_TYPE
    label: Access Type
    required: false
    type: enum
    multiple: true
    enum_values:
      - value: Push
        label: Push secret
      - value: Pull
        label: Pull secret
`)
	var file connectionTypeFile
	if err := yaml.Unmarshal(data, &file); err != nil {
		t.Fatalf("parse: %v", err)
	}

	spec := file.toIDCTSpec()
	if len(spec.CredentialsFields) != 1 {
		t.Fatalf("got %d fields, want 1", len(spec.CredentialsFields))
	}
	got := spec.CredentialsFields[0]
	if got.Type != fieldTypeEnum || !got.Multiple {
		t.Errorf("type=%q multiple=%v, want enum and multiple", got.Type, got.Multiple)
	}
	wantValues := []dchv1alpha1.EnumValue{
		{Value: testAccessTypePush, Label: testAccessTypePushLabel},
		{Value: testAccessTypePull, Label: testAccessTypePullLabel},
	}
	if !reflect.DeepEqual(got.EnumValues, wantValues) {
		t.Errorf("enum values = %+v, want %+v", got.EnumValues, wantValues)
	}

	ct := specToConnectionType(&spec)
	wantField := []EnumValue{
		{Value: testAccessTypePush, Label: testAccessTypePushLabel},
		{Value: testAccessTypePull, Label: testAccessTypePullLabel},
	}
	if f := ct.CredentialsFields[0]; !f.Multiple || !reflect.DeepEqual(f.EnumValues, wantField) {
		t.Errorf("REST field multiple=%v enum values=%+v, want true and %+v", f.Multiple, f.EnumValues, wantField)
	}
}
