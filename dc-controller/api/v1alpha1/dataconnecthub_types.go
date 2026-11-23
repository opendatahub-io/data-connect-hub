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
	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
)

// DataConnectHubSpec defines the platform configuration for Data Connect Hub.
// Only settings intentionally supported by the platform API are projected;
// application-specific configuration remains on DataConnectService.
type DataConnectHubSpec struct {
	// applicationsNamespace is where the platform-managed DataConnectService runs.
	// +kubebuilder:validation:MinLength=1
	ApplicationsNamespace string `json:"applicationsNamespace"`

	// distribution identifies the ODH or RHOAI distribution managing this module.
	// +optional
	Distribution dchv1alpha1.DistributionStatus `json:"distribution,omitempty"`

	// gateway selects the Gateway used by the platform-managed DataConnectService.
	// +optional
	Gateway *dchv1alpha1.Gateway `json:"gateway,omitempty"`

	// tokenReviewAudiences configures Kubernetes TokenReview authentication.
	// When omitted, the application controller's configured/default audiences apply.
	// +optional
	TokenReviewAudiences []string `json:"tokenReviewAudiences,omitempty"`
}

// DataConnectHubStatus defines the observed state of the platform module.
type DataConnectHubStatus struct {
	// observedGeneration is the last generation whose desired application state was applied.
	// +optional
	ObservedGeneration int64 `json:"observedGeneration,omitempty"`

	// distribution identifies the platform distribution context.
	// +optional
	Distribution dchv1alpha1.DistributionStatus `json:"distribution,omitempty"`

	// releases lists the module and application releases.
	// +optional
	Releases []dchv1alpha1.ReleaseStatus `json:"releases,omitempty"`

	// conditions represent the current state of the DataConnectHub resource.
	// +listType=map
	// +listMapKey=type
	// +optional
	Conditions []metav1.Condition `json:"conditions,omitempty"`
}

// +kubebuilder:object:root=true
// +kubebuilder:subresource:status
// +kubebuilder:resource:scope=Cluster,categories=opendatahub,shortName=dch
// +kubebuilder:validation:XValidation:rule="self.metadata.name == 'default-dataconnecthub'",message="DataConnectHub name must be default-dataconnecthub"
// +kubebuilder:printcolumn:name="Ready",type=string,JSONPath=`.status.conditions[?(@.type=="Ready")].status`,description="Ready"
// +kubebuilder:printcolumn:name="Distribution",type=string,JSONPath=`.status.distribution.name`,description="Platform distribution"
// DataConnectHub is the cluster-scoped singleton module API for Data Connect Hub.
// It is distinct from the namespaced DataConnectService application API.
type DataConnectHub struct {
	metav1.TypeMeta   `json:",inline"`
	metav1.ObjectMeta `json:"metadata,omitempty"`

	// spec defines the desired platform configuration.
	// +required
	Spec DataConnectHubSpec `json:"spec"`

	// status defines the observed state.
	// +optional
	Status DataConnectHubStatus `json:"status,omitempty"`
}

// +kubebuilder:object:root=true

// DataConnectHubList contains a list of DataConnectHub.
type DataConnectHubList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []DataConnectHub `json:"items"`
}

func init() {
	SchemeBuilder.Register(func(s *runtime.Scheme) error {
		s.AddKnownTypes(SchemeGroupVersion, &DataConnectHub{}, &DataConnectHubList{})
		return nil
	})
}
