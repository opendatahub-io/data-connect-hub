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
	"testing"

	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/resource"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	traceLogLevel = "trace"
	debugLogLevel = "debug"

	// testFlightServiceName is the expected flight resource name for a CR
	// named "default-dcs" (see flightServiceResourceName).
	testFlightServiceName = "default-dcs-flight"
)

func envNames(env []any) []string {
	names := make([]string, 0, len(env))
	for _, e := range env {
		entry, _ := e.(map[string]any)
		names = append(names, entry["name"].(string))
	}
	return names
}

func countRustLog(env []any) int {
	count := 0
	for _, n := range envNames(env) {
		if n == rustLogEnvName {
			count++
		}
	}
	return count
}

func TestBuildHelmValuesHTTPRouteName(t *testing.T) {
	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: "default-dcs"},
	}
	r := &DataConnectServiceReconciler{}
	values, err := r.buildHelmValues(cr, &platformConfig{}, RenderOptions{})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if values["httpRouteName"] != "dch-default-dcs-route" {
		t.Fatalf("expected httpRouteName derived from CR name, got %v", values["httpRouteName"])
	}
}

func TestWithDefaultEnv(t *testing.T) {
	existing := []any{
		map[string]any{keyName: rustLogEnvName, keyValue: debugLogLevel},
		map[string]any{keyName: "OTHER", keyValue: "x"},
	}
	got := withDefaultRustLogEnv(existing)
	if countRustLog(got) != 1 {
		t.Fatalf("expected no duplicate RUST_LOG, got %v", envNames(got))
	}
	if got[0].(map[string]any)["value"] != debugLogLevel {
		t.Fatalf("expected existing entry to be preserved, got %v", got[0])
	}

	got = withDefaultRustLogEnv(nil)
	if countRustLog(got) != 1 {
		t.Fatalf("expected default RUST_LOG to be added, got %v", envNames(got))
	}
	if got[0].(map[string]any)["value"] != defaultRustLogLevel {
		t.Fatalf("expected default value info, got %v", got[0])
	}
}

func TestBuildRestServiceValuesEnvDefault(t *testing.T) {
	cr := &dchv1alpha1.DataConnectService{}
	rest, err := buildRestServiceValues(cr, nil)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if countRustLog(rest["env"].([]any)) != 1 {
		t.Fatalf("expected exactly one default RUST_LOG, got %v", envNames(rest["env"].([]any)))
	}

	cr = &dchv1alpha1.DataConnectService{
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{
				Env: []corev1.EnvVar{{Name: rustLogEnvName, Value: debugLogLevel}},
			},
		},
	}
	rest, err = buildRestServiceValues(cr, nil)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	env := rest["env"].([]any)
	if countRustLog(env) != 1 {
		t.Fatalf("expected exactly one RUST_LOG, got %v", envNames(env))
	}
	if env[0].(map[string]any)["value"] != debugLogLevel {
		t.Fatalf("expected CR-provided value to win, got %v", env[0])
	}
}

func TestBuildFlightServiceValuesEnvDefault(t *testing.T) {
	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: "default-dcs"},
		Spec: dchv1alpha1.DataConnectServiceSpec{
			FlightService: &dchv1alpha1.FlightServiceConfig{},
		},
	}
	f, err := buildFlightServiceValues(cr, nil)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if f == nil {
		t.Fatal("expected flight values, got nil")
	}
	if f[keyName] != testFlightServiceName {
		t.Fatalf("expected flight name derived from CR name, got %v", f[keyName])
	}
	if countRustLog(f["env"].([]any)) != 1 {
		t.Fatalf("expected exactly one default RUST_LOG, got %v", envNames(f["env"].([]any)))
	}

	cr.Spec.FlightService.Env = []corev1.EnvVar{{Name: rustLogEnvName, Value: traceLogLevel}}
	f, err = buildFlightServiceValues(cr, nil)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	env := f["env"].([]any)
	if countRustLog(env) != 1 {
		t.Fatalf("expected exactly one RUST_LOG, got %v", envNames(env))
	}
	if env[0].(map[string]any)[keyValue] != traceLogLevel {
		t.Fatalf("expected CR-provided value to win, got %v", env[0])
	}

	// Omitting flightService in the spec still renders default flight
	// values, so Flight is always deployed.
	cr.Spec.FlightService = nil
	f, err = buildFlightServiceValues(cr, nil)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if f == nil {
		t.Fatal("expected default flight values, got nil")
	}
	if f[keyName] != testFlightServiceName {
		t.Fatalf("expected flight name derived from CR name, got %v", f[keyName])
	}
	if countRustLog(f["env"].([]any)) != 1 {
		t.Fatalf("expected exactly one default RUST_LOG, got %v", envNames(f["env"].([]any)))
	}
}

func TestValidateVolumesRejectsUnsafeSources(t *testing.T) {
	vols := []corev1.Volume{
		{Name: "data", VolumeSource: corev1.VolumeSource{
			HostPath: &corev1.HostPathVolumeSource{Path: "/host"},
		}},
		{Name: "nfs", VolumeSource: corev1.VolumeSource{
			NFS: &corev1.NFSVolumeSource{Server: "nfs.example.com", Path: "/export"},
		}},
		{Name: "flex", VolumeSource: corev1.VolumeSource{
			FlexVolume: &corev1.FlexVolumeSource{Driver: "example.com/driver"},
		}},
	}
	for _, v := range vols {
		if err := validateVolumes([]corev1.Volume{v}); err == nil {
			t.Fatalf("expected error for %q volume", v.Name)
		}
	}
}

func TestValidateVolumesAllowsSafeTypes(t *testing.T) {
	vols := []corev1.Volume{
		{Name: "cfg", VolumeSource: corev1.VolumeSource{
			ConfigMap: &corev1.ConfigMapVolumeSource{
				LocalObjectReference: corev1.LocalObjectReference{Name: "my-cm"},
			},
		}},
		{Name: "sec", VolumeSource: corev1.VolumeSource{
			Secret: &corev1.SecretVolumeSource{SecretName: "my-secret"},
		}},
		{Name: "scratch", VolumeSource: corev1.VolumeSource{
			EmptyDir: &corev1.EmptyDirVolumeSource{},
		}},
		{Name: "pvc", VolumeSource: corev1.VolumeSource{
			PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSource{ClaimName: "my-pvc"},
		}},
		{Name: "proj", VolumeSource: corev1.VolumeSource{
			Projected: &corev1.ProjectedVolumeSource{},
		}},
		{Name: "downward", VolumeSource: corev1.VolumeSource{
			DownwardAPI: &corev1.DownwardAPIVolumeSource{},
		}},
		{Name: "csi", VolumeSource: corev1.VolumeSource{
			CSI: &corev1.CSIVolumeSource{Driver: "csi.example.com"},
		}},
		{Name: "ephemeral", VolumeSource: corev1.VolumeSource{
			Ephemeral: &corev1.EphemeralVolumeSource{},
		}},
	}
	if err := validateVolumes(vols); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestToUnstructuredSlicePreservesAllFields(t *testing.T) {
	optional := true
	envs := []corev1.EnvVar{
		{Name: "FROM_SECRET", ValueFrom: &corev1.EnvVarSource{
			SecretKeyRef: &corev1.SecretKeySelector{
				LocalObjectReference: corev1.LocalObjectReference{Name: "s"},
				Key:                  "k",
				Optional:             &optional,
			},
		}},
		{Name: "FROM_FIELD", ValueFrom: &corev1.EnvVarSource{
			ResourceFieldRef: &corev1.ResourceFieldSelector{
				Resource: "limits.cpu",
			},
		}},
	}

	result, err := toUnstructuredSlice(envs)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(result) != 2 {
		t.Fatalf("expected 2 items, got %d", len(result))
	}

	// Verify optional field is preserved
	entry0 := result[0].(map[string]any)
	vf := entry0["valueFrom"].(map[string]any)
	skr := vf["secretKeyRef"].(map[string]any)
	if skr["optional"] != true {
		t.Fatalf("expected optional=true to be preserved, got %v", skr["optional"])
	}

	// Verify resourceFieldRef is preserved
	entry1 := result[1].(map[string]any)
	vf1 := entry1["valueFrom"].(map[string]any)
	if _, ok := vf1["resourceFieldRef"]; !ok {
		t.Fatal("expected resourceFieldRef to be preserved")
	}
}

func TestResourcesPreservesEphemeralStorage(t *testing.T) {
	cr := &dchv1alpha1.DataConnectService{
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{
				Resources: &corev1.ResourceRequirements{
					Requests: corev1.ResourceList{
						corev1.ResourceCPU:              resource.MustParse("100m"),
						corev1.ResourceEphemeralStorage: resource.MustParse("1Gi"),
					},
				},
			},
		},
	}
	rest, err := buildRestServiceValues(cr, nil)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	res := rest[keyResources].(map[string]any)
	requests := res["requests"].(map[string]any)
	if _, ok := requests["ephemeral-storage"]; !ok {
		t.Fatal("expected ephemeral-storage to be preserved in resources")
	}
}

func TestHostPathVolumeRejectedInRestService(t *testing.T) {
	cr := &dchv1alpha1.DataConnectService{
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{
				Volumes: []corev1.Volume{
					{Name: "bad", VolumeSource: corev1.VolumeSource{
						HostPath: &corev1.HostPathVolumeSource{Path: "/host"},
					}},
				},
			},
		},
	}
	_, err := buildRestServiceValues(cr, nil)
	if err == nil {
		t.Fatal("expected error for hostPath volume in rest service")
	}
}
