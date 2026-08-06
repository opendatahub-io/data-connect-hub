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
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
)

// Gateway identifies a Kubernetes Gateway resource by name and namespace.
type Gateway struct {
	// name is the name of the Gateway resource
	Name string `json:"name"`

	// namespace is the namespace of the Gateway resource
	Namespace string `json:"namespace"`
}

// ServiceOverrides allows per-service customisation of image, scaling, and pod spec fields.
type ServiceOverrides struct {
	// image overrides the container image for this service
	// +optional
	Image *string `json:"image,omitempty"`

	// replicas overrides the number of pods
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:default=1
	// +optional
	Replicas *int32 `json:"replicas,omitempty"`

	// resources overrides the container resource requirements
	// +optional
	Resources *corev1.ResourceRequirements `json:"resources,omitempty"`

	// env is a list of additional environment variables to set on the container
	// +optional
	Env []corev1.EnvVar `json:"env,omitempty"`

	// envFrom is a list of sources to populate environment variables
	// +optional
	EnvFrom []corev1.EnvFromSource `json:"envFrom,omitempty"`

	// volumes is a list of additional volumes to add to the pod
	// +optional
	Volumes []corev1.Volume `json:"volumes,omitempty"`

	// volumeMounts is a list of additional volume mounts to add to the container
	// +optional
	VolumeMounts []corev1.VolumeMount `json:"volumeMounts,omitempty"`

	// imagePullSecrets is a list of references to secrets for pulling the container image
	// +optional
	ImagePullSecrets []corev1.LocalObjectReference `json:"imagePullSecrets,omitempty"`
}

// DatabaseSpec configures the database backend for the DataConnectService.
type DatabaseSpec struct {
	// devMode when true deploys a built-in single-instance Postgres.
	// When false, the controller expects the user to provide an external database
	// via externalSecret.
	// +kubebuilder:default=true
	// +optional
	DevMode *bool `json:"devMode,omitempty"`

	// externalSecret is the name of a Secret containing database connection details.
	// Used when devMode is false.
	// +optional
	ExternalSecret *string `json:"externalSecret,omitempty"`
}

// DataConnectServiceSpec defines the desired state of DataConnectService
type DataConnectServiceSpec struct {
	// description is a human-readable description of the service
	// +optional
	Description string `json:"description,omitempty"`

	// restService configures the REST API deployment
	// +optional
	RestService *ServiceOverrides `json:"restService,omitempty"`

	// flightService configures the Flight gRPC API deployment
	// +optional
	FlightService *ServiceOverrides `json:"flightService,omitempty"`

	// database configures the database backend
	// +optional
	Database *DatabaseSpec `json:"database,omitempty"`

	// gateway is a reference to a Kubernetes Gateway for external traffic.
	// Defaults to the ODH gateway (odh-gateway in opendatahub namespace).
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
