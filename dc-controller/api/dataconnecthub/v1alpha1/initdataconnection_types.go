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

// SecretRef is a reference to a Kubernetes Secret in the same namespace.
type SecretRef struct {
	// name is the name of the Secret
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=253
	Name string `json:"name"`
}

// InitDataConnectionSpec defines the desired state of InitDataConnection.
type InitDataConnectionSpec struct {
	// name is the human-readable display name for the connection
	// +kubebuilder:validation:MaxLength=253
	Name string `json:"name"`

	// connectionTypeName references the InitDataConnectionType by spec.name
	// (e.g. "S3", "Postgres")
	// +kubebuilder:validation:MaxLength=253
	ConnectionTypeName string `json:"connectionTypeName"`

	// format is the data format of the connection
	// +kubebuilder:validation:Enum=tabular;binary
	// +kubebuilder:default=tabular
	// +optional
	Format string `json:"format,omitempty"`

	// secretRef points to a Kubernetes Secret in the same namespace
	// containing the connection credentials. The Secret data keys
	// must match the credential field names defined by the connection type.
	// The Secret is never copied — only this reference is stored.
	SecretRef SecretRef `json:"secretRef"`

	// properties holds optional non-secret metadata for the connection
	// +kubebuilder:validation:MaxProperties=64
	// +optional
	Properties map[string]string `json:"properties,omitempty"`
}

// InitDataConnectionStatus defines the observed state of InitDataConnection.
type InitDataConnectionStatus struct {
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
// +kubebuilder:resource:categories=opendatahub,shortName=idc
// +kubebuilder:printcolumn:name="Type",type=string,JSONPath=`.spec.connectionTypeName`
// +kubebuilder:printcolumn:name="Secret",type=string,JSONPath=`.spec.secretRef.name`
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=`.metadata.creationTimestamp`

// InitDataConnection is the Schema for the initdataconnections API.
// It declares a data connection for GitOps-driven provisioning.
// The controller registers the connection with the DCH REST service,
// storing only a reference to the credentials Secret — the Secret
// data is never persisted in the database.
type InitDataConnection struct {
	metav1.TypeMeta `json:",inline"`

	// metadata is a standard object metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitzero"`

	// spec defines the desired state of InitDataConnection
	// +required
	Spec InitDataConnectionSpec `json:"spec"`

	// status defines the observed state of InitDataConnection
	// +optional
	Status InitDataConnectionStatus `json:"status,omitzero"`
}

// +kubebuilder:object:root=true

// InitDataConnectionList contains a list of InitDataConnection
type InitDataConnectionList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitzero"`
	Items           []InitDataConnection `json:"items"`
}

func init() {
	SchemeBuilder.Register(func(s *runtime.Scheme) error {
		s.AddKnownTypes(SchemeGroupVersion, &InitDataConnection{}, &InitDataConnectionList{})
		return nil
	})
}
