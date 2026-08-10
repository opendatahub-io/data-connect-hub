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

// DatabaseSpec configures an external database backend for the DataConnectHub.
// Only used when devMode is false.
type DatabaseSpec struct {
	// externalSecret is the name of a Secret containing database connection details.
	// +optional
	ExternalSecret *string `json:"externalSecret,omitempty"`
}

// DistributionStatus identifies the platform distribution context.
type DistributionStatus struct {
	// name is the distribution name (e.g., SelfManagedRHOAI, OpenDataHub, Standalone)
	// +optional
	Name string `json:"name,omitempty"`

	// version is the distribution version (e.g., 3.5.1, 0.0.0)
	// +optional
	Version string `json:"version,omitempty"`
}

// ReleaseStatus describes a deployed component release.
type ReleaseStatus struct {
	// name is the name of the component
	// +optional
	Name string `json:"name,omitempty"`

	// repoUrl is the repository URL of the component
	// +optional
	RepoUrl string `json:"repoUrl,omitempty"`

	// version is the version of the component
	// +optional
	Version string `json:"version,omitempty"`
}

// DataConnectHubSpec defines the desired state of DataConnectHub
// +kubebuilder:validation:XValidation:rule="self.devMode != false || (has(self.database) && has(self.database.externalSecret))",message="database.externalSecret is required when devMode is false"
type DataConnectHubSpec struct {
	// devMode when true deploys a built-in single-instance Postgres.
	// When false, the user provides an external database via the database field.
	// +kubebuilder:default=true
	// +optional
	DevMode *bool `json:"devMode,omitempty"`

	// restService configures the REST API deployment
	// +optional
	RestService *ServiceOverrides `json:"restService,omitempty"`

	// flightService configures the Flight gRPC API deployment
	// +optional
	FlightService *ServiceOverrides `json:"flightService,omitempty"`

	// database configures the external database backend (used when devMode is false)
	// +optional
	Database *DatabaseSpec `json:"database,omitempty"`

	// gateway is a reference to a Kubernetes Gateway for external traffic.
	// Defaults to the ODH gateway (odh-gateway in opendatahub namespace).
	// +optional
	Gateway *Gateway `json:"gateway,omitempty"`
}

// DataConnectHubStatus defines the observed state of DataConnectHub.
type DataConnectHubStatus struct {
	// observedGeneration is the last generation observed by the controller
	// +optional
	ObservedGeneration int64 `json:"observedGeneration,omitempty"`

	// distribution identifies the platform distribution context
	// +optional
	Distribution DistributionStatus `json:"distribution,omitempty"`

	// releases lists the deployed component versions
	// +optional
	Releases []ReleaseStatus `json:"releases,omitempty"`

	// phase represents the current lifecycle phase of the DataConnectHub
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

	// conditions represent the current state of the DataConnectHub resource.
	// +listType=map
	// +listMapKey=type
	// +optional
	Conditions []metav1.Condition `json:"conditions,omitempty"`
}

// +kubebuilder:object:root=true
// +kubebuilder:subresource:status
// +kubebuilder:resource:scope=Cluster,categories=opendatahub,shortName=dch
// +kubebuilder:printcolumn:name="Phase",type=string,JSONPath=`.status.phase`
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=`.metadata.creationTimestamp`
// +kubebuilder:validation:XValidation:rule="self.metadata.name == 'default-dataconnecthub'",message="DataConnectHub must be named 'default-dataconnecthub'"

// DataConnectHub is the Schema for the dataconnecthubs API
type DataConnectHub struct {
	metav1.TypeMeta `json:",inline"`

	// metadata is a standard object metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitzero"`

	// spec defines the desired state of DataConnectHub
	// +required
	Spec DataConnectHubSpec `json:"spec"`

	// status defines the observed state of DataConnectHub
	// +optional
	Status DataConnectHubStatus `json:"status,omitzero"`
}

// +kubebuilder:object:root=true

// DataConnectHubList contains a list of DataConnectHub
type DataConnectHubList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitzero"`
	Items           []DataConnectHub `json:"items"`
}

func init() {
	SchemeBuilder.Register(func(s *runtime.Scheme) error {
		s.AddKnownTypes(SchemeGroupVersion, &DataConnectHub{}, &DataConnectHubList{})
		return nil
	})
}
