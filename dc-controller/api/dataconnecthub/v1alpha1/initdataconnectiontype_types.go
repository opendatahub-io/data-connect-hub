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

package v1alpha1

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
)

// EnumValue represents a selectable option for an enumerated field.
type EnumValue struct {
	// value is the internal value of the option
	// +kubebuilder:validation:MaxLength=256
	Value string `json:"value"`

	// label is the human-readable label for the option
	// +kubebuilder:validation:MaxLength=256
	Label string `json:"label"`
}

// CredentialsField describes a single field in a connection type's credentials form.
type CredentialsField struct {
	// name is the field identifier
	// +kubebuilder:validation:MaxLength=253
	Name string `json:"name"`

	// label is the human-readable label for the field
	// +kubebuilder:validation:MaxLength=256
	Label string `json:"label"`

	// description is an optional help text for the field
	// +kubebuilder:validation:MaxLength=1024
	// +optional
	Description *string `json:"description,omitempty"`

	// required indicates whether the field must be provided
	Required bool `json:"required"`

	// type is the data type of the field
	// +kubebuilder:validation:Enum=string;enum
	Type string `json:"type"`

	// enumValues lists the allowed values when type is "enum"
	// +optional
	EnumValues []EnumValue `json:"enumValues,omitempty"`

	// defaultValue is the default value for the field
	// +kubebuilder:validation:MaxLength=1024
	// +optional
	DefaultValue *string `json:"defaultValue,omitempty"`
}

// InitDataConnectionTypeSpec defines the desired state of InitDataConnectionType.
type InitDataConnectionTypeSpec struct {
	// name is the unique name of the connection type
	// +kubebuilder:validation:MaxLength=253
	Name string `json:"name"`

	// provider identifies the backing data provider (e.g. "postgres", "s3")
	// +kubebuilder:validation:MaxLength=253
	Provider string `json:"provider"`

	// description is an optional human-readable description
	// +kubebuilder:validation:MaxLength=1024
	// +optional
	Description *string `json:"description,omitempty"`

	// credentialsFields defines the fields required to configure credentials for this connection type
	CredentialsFields []CredentialsField `json:"credentialsFields"`
}

// InitDataConnectionTypeStatus defines the observed state of InitDataConnectionType.
type InitDataConnectionTypeStatus struct {
	// phase represents the current lifecycle phase
	// +optional
	Phase string `json:"phase,omitempty"`

	// conditions represent the current state of the resource
	// +listType=map
	// +listMapKey=type
	// +optional
	Conditions []metav1.Condition `json:"conditions,omitempty"`
}

// +kubebuilder:object:root=true
// +kubebuilder:subresource:status
// +kubebuilder:resource:categories=opendatahub,shortName=idct
// +kubebuilder:printcolumn:name="Provider",type=string,JSONPath=`.spec.provider`
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=`.metadata.creationTimestamp`

// InitDataConnectionType is the Schema for the initdataconnectiontypes API
type InitDataConnectionType struct {
	metav1.TypeMeta `json:",inline"`

	// metadata is a standard object metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitzero"`

	// spec defines the desired state of InitDataConnectionType
	// +required
	Spec InitDataConnectionTypeSpec `json:"spec"`

	// status defines the observed state of InitDataConnectionType
	// +optional
	Status InitDataConnectionTypeStatus `json:"status,omitzero"`
}

// +kubebuilder:object:root=true

// InitDataConnectionTypeList contains a list of InitDataConnectionType
type InitDataConnectionTypeList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitzero"`
	Items           []InitDataConnectionType `json:"items"`
}

func init() {
	SchemeBuilder.Register(func(s *runtime.Scheme) error {
		s.AddKnownTypes(SchemeGroupVersion, &InitDataConnectionType{}, &InitDataConnectionTypeList{})
		return nil
	})
}
