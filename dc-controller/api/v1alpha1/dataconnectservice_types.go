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

// EDIT THIS FILE!  THIS IS SCAFFOLDING FOR YOU TO OWN!
// NOTE: json tags are required.  Any new fields you add must have json tags for the fields to be serialized.

// Gateway identifies a Kubernetes Gateway resource by name and namespace.
type Gateway struct {
	// name is the name of the Gateway resource
	Name string `json:"name"`

	// namespace is the namespace of the Gateway resource
	Namespace string `json:"namespace"`
}

// DataConnectServiceSpec defines the desired state of DataConnectService
type DataConnectServiceSpec struct {
	// description is a human-readable description of the service
	// +optional
	Description string `json:"description,omitempty"`

	// restApiReplicas is the number of replicas for the REST API deployment
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:default=1
	// +optional
	RestApiReplicas *int32 `json:"restApiReplicas,omitempty"`

	// flightApiReplicas is the number of replicas for the Flight gRPC API deployment
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:default=1
	// +optional
	FlightApiReplicas *int32 `json:"flightApiReplicas,omitempty"`

	// gateway is an optional reference to a Kubernetes Gateway for external traffic
	// +optional
	Gateway *Gateway `json:"gateway,omitempty"`
}

// DataConnectServiceStatus defines the observed state of DataConnectService.
type DataConnectServiceStatus struct {
	// phase represents the current lifecycle phase of the DataConnectService
	// +optional
	Phase string `json:"phase,omitempty"`

	// hostname is the hostname where the service is reachable
	// +optional
	Hostname string `json:"hostname,omitempty"`

	// httpRoute is the name of the HTTPRoute resource created for this service
	// +optional
	HttpRoute string `json:"httpRoute,omitempty"`

	// gateway is the Gateway resource this service is attached to
	// +optional
	Gateway *Gateway `json:"gateway,omitempty"`

	// conditions represent the current state of the DataConnectService resource.
	// +listType=map
	// +listMapKey=type
	// +optional
	Conditions []metav1.Condition `json:"conditions,omitempty"`
}

// +kubebuilder:object:root=true
// +kubebuilder:subresource:status

// DataConnectService is the Schema for the dataconnectservices API
type DataConnectService struct {
	metav1.TypeMeta `json:",inline"`

	// metadata is a standard object metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitzero"`

	// spec defines the desired state of DataConnectService
	// +required
	Spec DataConnectServiceSpec `json:"spec"`

	// status defines the observed state of DataConnectService
	// +optional
	Status DataConnectServiceStatus `json:"status,omitzero"`
}

// +kubebuilder:object:root=true

// DataConnectServiceList contains a list of DataConnectService
type DataConnectServiceList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitzero"`
	Items           []DataConnectService `json:"items"`
}

func init() {
	SchemeBuilder.Register(func(s *runtime.Scheme) error {
		s.AddKnownTypes(SchemeGroupVersion, &DataConnectService{}, &DataConnectServiceList{})
		return nil
	})
}
