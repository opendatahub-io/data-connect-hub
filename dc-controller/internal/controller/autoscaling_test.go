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
	"strings"
	"testing"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/utils/ptr"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

func TestValidateAutoscalingRejectsReplicaOverride(t *testing.T) {
	t.Parallel()

	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: "example"},
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{
				Replicas: ptr.To(int32(1)),
				AutoScaling: &dchv1alpha1.AutoScalingConfig{
					MinReplicas: ptr.To(int32(1)),
					MaxReplicas: ptr.To(int32(2)),
					Triggers:    []dchv1alpha1.AutoscalingTrigger{validAutoscalingTrigger()},
				},
			},
		},
	}

	if err := validateAutoscaling(cr); err == nil || !strings.Contains(err.Error(), "replicas") {
		t.Fatalf("validateAutoscaling() error = %v, want replica conflict", err)
	}
}

func TestAutoscalingResourcesRenderRESTAndFlightTargets(t *testing.T) {
	t.Parallel()

	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: "default-dataconnectservice", Namespace: "dch-services"},
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{AutoScaling: validAutoscalingConfig()},
			FlightService: &dchv1alpha1.FlightServiceConfig{ServiceOverrides: dchv1alpha1.ServiceOverrides{
				AutoScaling: validAutoscalingConfig(),
			}},
		},
	}

	resources := autoscalingResources(cr)

	var scaledObjects []*unstructured.Unstructured
	for _, resource := range resources {
		if resource.GetKind() == "ScaledObject" {
			scaledObjects = append(scaledObjects, resource)
		}
	}
	if len(scaledObjects) != 2 {
		t.Fatalf("rendered %d ScaledObjects, want 2", len(scaledObjects))
	}

	targets := map[string]string{}
	for _, scaledObject := range scaledObjects {
		name, _, err := unstructured.NestedString(scaledObject.Object, "spec", "scaleTargetRef", "name")
		if err != nil {
			t.Fatalf("reading target for %s: %v", scaledObject.GetName(), err)
		}
		targets[name] = scaledObject.GetName()
	}
	if _, ok := targets[renderedRestDeploymentName]; !ok {
		t.Errorf("REST deployment target %q was not rendered", renderedRestDeploymentName)
	}
	if _, ok := targets[renderedFlightDeploymentName(cr.Name)]; !ok {
		t.Errorf("Flight deployment target %q was not rendered", renderedFlightDeploymentName(cr.Name))
	}

	for _, scaledObject := range scaledObjects {
		triggers, found, err := unstructured.NestedSlice(scaledObject.Object, "spec", "triggers")
		if err != nil || !found || len(triggers) == 0 {
			t.Fatalf("reading trigger metadata for %s: found=%t error=%v", scaledObject.GetName(), found, err)
		}
		metadata, found, err := unstructured.NestedStringMap(triggers[0].(map[string]any), "metadata")
		if err != nil || !found {
			t.Fatalf("reading trigger metadata for %s: found=%t error=%v", scaledObject.GetName(), found, err)
		}
		if metadata["namespace"] != cr.Namespace {
			t.Errorf("trigger namespace = %q, want %q", metadata["namespace"], cr.Namespace)
		}
	}
}

func TestValidateAutoscalingRequiresServerAddress(t *testing.T) {
	t.Parallel()

	trigger := validAutoscalingTrigger()
	trigger.ServerAddress = ""
	cr := &dchv1alpha1.DataConnectService{Spec: dchv1alpha1.DataConnectServiceSpec{
		RestService: &dchv1alpha1.ServiceOverrides{AutoScaling: &dchv1alpha1.AutoScalingConfig{
			MinReplicas: ptr.To(int32(1)),
			MaxReplicas: ptr.To(int32(2)),
			Triggers:    []dchv1alpha1.AutoscalingTrigger{trigger},
		}},
	}}

	if err := validateAutoscaling(cr); err == nil || !strings.Contains(err.Error(), "serverAddress") {
		t.Fatalf("validateAutoscaling() error = %v, want serverAddress validation", err)
	}
}

func validAutoscalingConfig() *dchv1alpha1.AutoScalingConfig {
	return &dchv1alpha1.AutoScalingConfig{
		MinReplicas: ptr.To(int32(1)),
		MaxReplicas: ptr.To(int32(3)),
		Triggers:    []dchv1alpha1.AutoscalingTrigger{validAutoscalingTrigger()},
	}
}

func validAutoscalingTrigger() dchv1alpha1.AutoscalingTrigger {
	return dchv1alpha1.AutoscalingTrigger{
		Name:          "request-rate",
		ServerAddress: "https://thanos.example.test",
		Query:         "sum(rate(requests_total[2m]))",
		Target:        dchv1alpha1.AutoscalingTarget{Type: "Value", Value: "0.4"},
	}
}
