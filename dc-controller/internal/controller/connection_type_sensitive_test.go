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
	"testing"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/yaml"
)

// Credential field names used by the test fixtures and expectations.
const (
	testFieldURI      = "URI"
	testFieldPassword = "PASSWORD"
	testFieldToken    = "TOKEN"
	testFieldAPIKey   = "API_KEY"

	testFieldAWSAccessKeyID     = "AWS_ACCESS_KEY_ID"
	testFieldAWSSecretAccessKey = "AWS_SECRET_ACCESS_KEY"
	testFieldAWSS3Endpoint      = "AWS_S3_ENDPOINT"
	testFieldAccessType         = "ACCESS_TYPE"
	testFieldDockerConfigJSON   = ".dockerconfigjson"
)

// builtinSensitiveFields lists credential field names that must be marked
// sensitive in every built-in connection type that defines them. URI is
// included because connection URIs may embed passwords.
var builtinSensitiveFields = map[string]bool{
	testFieldURI:      true,
	testFieldPassword: true,
	testFieldToken:    true,
	testFieldAPIKey:   true,
}

func TestBuiltinConnectionTypesSensitiveFields(t *testing.T) {
	typesDir := filepath.Join("..", "..", "..", "config", "connection-types")
	entries, err := os.ReadDir(typesDir)
	if err != nil {
		t.Fatalf("read %s: %v", typesDir, err)
	}

	checked := 0
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

		spec := file.toIDCTSpec()
		ct := specToConnectionType(&spec)
		for _, f := range ct.CredentialsFields {
			want := builtinSensitiveFields[f.Name]
			if f.Sensitive != want {
				t.Errorf("%s: field %s sensitive=%v, want %v", entry.Name(), f.Name, f.Sensitive, want)
			}
			checked++
		}
	}
	if checked == 0 {
		t.Fatalf("no credential fields found under %s", typesDir)
	}
}

func TestConfigMapToConnectionTypeSensitive(t *testing.T) {
	// Fields adapted from the RHOAI Dashboard s3, oci-v1, and uri-v1 ConfigMaps.
	// The dashboard's "hidden" and "file" types map to sensitive; "uri" fields do
	// not, even though the built-in URI connection type marks its URI field sensitive.
	fieldsJSON := `[
		{"type":"short-text","name":"Access key","envVar":"AWS_ACCESS_KEY_ID","properties":{},"required":true},
		{"type":"hidden","name":"Secret key","envVar":"AWS_SECRET_ACCESS_KEY","required":true,"properties":{"deferInput":true}},
		{"type":"short-text","name":"Endpoint","envVar":"AWS_S3_ENDPOINT","required":true,"properties":{}},
		{"type":"dropdown","name":"Access type","envVar":"ACCESS_TYPE","properties":{"variant":"multi","items":[{"label":"Push secret","value":"Push"}]}},
		{"type":"file","name":"Secret details","envVar":".dockerconfigjson","properties":{"deferInput":true}},
		{"type":"uri","name":"URI","envVar":"URI","required":true,"properties":{}}
	]`
	cm := &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "s3"}}

	ct, err := configMapToConnectionType(cm, fieldsJSON)
	if err != nil {
		t.Fatalf("configMapToConnectionType: %v", err)
	}

	want := map[string]bool{
		testFieldAWSAccessKeyID:     false,
		testFieldAWSSecretAccessKey: true,
		testFieldAWSS3Endpoint:      false,
		testFieldAccessType:         false,
		testFieldDockerConfigJSON:   true,
		testFieldURI:                false,
	}
	if len(ct.CredentialsFields) != len(want) {
		t.Fatalf("got %d fields, want %d", len(ct.CredentialsFields), len(want))
	}
	for _, f := range ct.CredentialsFields {
		if f.Sensitive != want[f.Name] {
			t.Errorf("field %s sensitive=%v, want %v", f.Name, f.Sensitive, want[f.Name])
		}
	}
}
