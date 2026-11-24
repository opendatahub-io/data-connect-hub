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
	"reflect"
	"testing"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	testTypeOCIv1      = "oci-v1"
	testFieldModeName  = "MODE"
	testFieldModeLabel = "Mode"
)

func strPtr(s string) *string { return &s }

// TestConfigMapToConnectionTypeOCIv1 uses the oci-v1 fields shipped by the
// RHOAI 3.5 Dashboard: a multi-select dropdown with a description, and a file
// field that only has properties.helperText.
func TestConfigMapToConnectionTypeOCIv1(t *testing.T) {
	fieldsJSON := `[
		{"type":"dropdown","name":"Access type","description":"Use the push access type for uploading data, pull for accessing resources, or both.","envVar":"ACCESS_TYPE","properties":{"variant":"multi","items":[{"label":"Push secret","value":"Push"},{"label":"Pull secret","value":"Pull"}]}},
		{"type":"file","name":"Secret details","envVar":".dockerconfigjson","properties":{"deferInput":true,"extensions":[".dockerconfigjson",".json"],"helperText":"Secret details are visible to users with access to the project."}},
		{"type":"short-text","name":"Registry host","description":"The URL of the registry endpoint.","envVar":"OCI_HOST","properties":{}}
	]`
	cm := &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: testTypeOCIv1}}

	ct, err := configMapToConnectionType(cm, fieldsJSON)
	if err != nil {
		t.Fatalf("configMapToConnectionType: %v", err)
	}

	want := []Field{
		{
			Name:        testFieldAccessType,
			Label:       "Access type",
			Description: strPtr("Use the push access type for uploading data, pull for accessing resources, or both."),
			Type:        fieldTypeEnum,
			EnumValues: []EnumValue{
				{Value: "Push", Label: "Push secret"},
				{Value: "Pull", Label: "Pull secret"},
			},
			Multiple: true,
		},
		{
			Name:        ".dockerconfigjson",
			Label:       "Secret details",
			Description: strPtr("Secret details are visible to users with access to the project."),
			Type:        fieldTypeString,
			Sensitive:   true,
		},
		{
			Name:        "OCI_HOST",
			Label:       "Registry host",
			Description: strPtr("The URL of the registry endpoint."),
			Type:        fieldTypeString,
		},
	}
	if !reflect.DeepEqual(ct.CredentialsFields, want) {
		t.Errorf("credentials fields mismatch\n got: %+v\nwant: %+v", ct.CredentialsFields, want)
	}
}

func TestOdhFieldToField(t *testing.T) {
	tests := []struct {
		name string
		in   odhField
		want Field
	}{
		{
			name: "description wins over helperText",
			in: odhField{
				Type: "short-text", Name: testFieldLabel, EnvVar: testFieldName, Description: "desc",
				Properties: odhFieldProperties{HelperText: "helper"},
			},
			want: Field{Name: testFieldName, Label: testFieldLabel, Description: strPtr("desc"), Type: fieldTypeString},
		},
		{
			name: "no description or helperText",
			in:   odhField{Type: "short-text", Name: testFieldLabel, EnvVar: testFieldName, Required: true},
			want: Field{Name: testFieldName, Label: testFieldLabel, Required: true, Type: fieldTypeString},
		},
		{
			name: "single-select dropdown",
			in: odhField{
				Type: odhFieldTypeDropdown, Name: testFieldModeLabel, EnvVar: testFieldModeName,
				Properties: odhFieldProperties{Variant: "single", Items: []odhDropdownItem{{Label: "Read", Value: "r"}}},
			},
			want: Field{
				Name: testFieldModeName, Label: testFieldModeLabel, Type: fieldTypeEnum,
				EnumValues: []EnumValue{{Value: "r", Label: "Read"}},
			},
		},
		{
			name: "dropdown items without value are dropped, empty label uses value",
			in: odhField{
				Type: odhFieldTypeDropdown, Name: testFieldModeLabel, EnvVar: testFieldModeName,
				Properties: odhFieldProperties{Items: []odhDropdownItem{{Label: "Blank"}, {Value: "w"}}},
			},
			want: Field{
				Name: testFieldModeName, Label: testFieldModeLabel, Type: fieldTypeEnum,
				EnumValues: []EnumValue{{Value: "w", Label: "w"}},
			},
		},
		{
			name: "dropdown without items falls back to string",
			in: odhField{
				Type: odhFieldTypeDropdown, Name: testFieldModeLabel, EnvVar: testFieldModeName,
				Properties: odhFieldProperties{Variant: odhDropdownVariantMulti},
			},
			want: Field{Name: testFieldModeName, Label: testFieldModeLabel, Type: fieldTypeString},
		},
		{
			name: "hidden is sensitive",
			in:   odhField{Type: odhFieldTypeHidden, Name: "Secret key", EnvVar: "SECRET"},
			want: Field{Name: "SECRET", Label: "Secret key", Type: fieldTypeString, Sensitive: true},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := odhFieldToField(tt.in)
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("got %+v, want %+v", got, tt.want)
			}
		})
	}
}

func TestConfigMapToConnectionTypeSkipsSections(t *testing.T) {
	fieldsJSON := `[
		{"type":"section","name":"Authentication","description":"Credentials for the endpoint"},
		{"type":"short-text","name":"Host","envVar":"HOST","properties":{}}
	]`
	cm := &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "custom"}}

	ct, err := configMapToConnectionType(cm, fieldsJSON)
	if err != nil {
		t.Fatalf("configMapToConnectionType: %v", err)
	}
	if len(ct.CredentialsFields) != 1 || ct.CredentialsFields[0].Name != testFieldName {
		t.Errorf("want only HOST field, got %+v", ct.CredentialsFields)
	}
}

func TestSpecToConnectionTypeMultiple(t *testing.T) {
	spec := dchv1alpha1.InitDataConnectionTypeSpec{
		Name:     testTypeOCIv1,
		Provider: testTypeOCIv1,
		CredentialsFields: []dchv1alpha1.CredentialsField{{
			Name:       testFieldAccessType,
			Label:      "Access type",
			Type:       fieldTypeEnum,
			EnumValues: []dchv1alpha1.EnumValue{{Value: "Push", Label: "Push secret"}},
			Multiple:   true,
		}},
	}
	ct := specToConnectionType(&spec)
	if !ct.CredentialsFields[0].Multiple {
		t.Errorf("multiple not propagated from IDCT spec")
	}
}
