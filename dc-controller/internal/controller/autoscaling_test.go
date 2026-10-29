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
	"context"
	"errors"
	"strings"
	"testing"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/runtime/serializer"
	"k8s.io/apimachinery/pkg/types"
	clientgotesting "k8s.io/client-go/testing"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
	"sigs.k8s.io/controller-runtime/pkg/client/interceptor"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// autoscalingTestCRName matches the singleton DataConnectService name used
// across the controller tests.
const autoscalingTestCRName = "default-dataconnectservice"

// attackerTokenValue is the foreign bearer token planted in conflicting
// Secret seeds.
const attackerTokenValue = "attacker-bearer-token"

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
		ObjectMeta: metav1.ObjectMeta{Name: autoscalingTestCRName, Namespace: "dch-services"},
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

func restAutoscalingCRWithAddress(address string) *dchv1alpha1.DataConnectService {
	trigger := validAutoscalingTrigger()
	trigger.ServerAddress = address
	return &dchv1alpha1.DataConnectService{Spec: dchv1alpha1.DataConnectServiceSpec{
		RestService: &dchv1alpha1.ServiceOverrides{AutoScaling: &dchv1alpha1.AutoScalingConfig{
			MinReplicas: ptr.To(int32(1)),
			MaxReplicas: ptr.To(int32(2)),
			Triggers:    []dchv1alpha1.AutoscalingTrigger{trigger},
		}},
	}}
}

func flightAutoscalingCRWithAddress(address string) *dchv1alpha1.DataConnectService {
	trigger := validAutoscalingTrigger()
	trigger.ServerAddress = address
	return &dchv1alpha1.DataConnectService{Spec: dchv1alpha1.DataConnectServiceSpec{
		FlightService: &dchv1alpha1.FlightServiceConfig{ServiceOverrides: dchv1alpha1.ServiceOverrides{
			AutoScaling: &dchv1alpha1.AutoScalingConfig{
				MinReplicas: ptr.To(int32(1)),
				MaxReplicas: ptr.To(int32(2)),
				Triggers:    []dchv1alpha1.AutoscalingTrigger{trigger},
			},
		}},
	}}
}

func TestValidateAutoscalingAcceptsBuiltInMonitoringEndpoint(t *testing.T) {
	t.Parallel()

	if err := validateAutoscaling(restAutoscalingCRWithAddress(autoscalingMonitoringEndpoint)); err != nil {
		t.Fatalf("validateAutoscaling() error = %v, want nil", err)
	}
}

func TestValidateAutoscalingServerAddressIsCaseAndWhitespaceInsensitive(t *testing.T) {
	t.Parallel()

	address := "  https://THANOS-QUERIER.openshift-monitoring.svc.cluster.local:9092  "
	if err := validateAutoscaling(restAutoscalingCRWithAddress(address)); err != nil {
		t.Fatalf("validateAutoscaling() error = %v, want nil", err)
	}
}

func TestValidateAutoscalingRejectsPlainHTTPServerAddress(t *testing.T) {
	t.Parallel()

	for _, address := range []string{
		"http://thanos-querier.openshift-monitoring.svc.cluster.local:9092",
		"http://attacker.example",
	} {
		err := validateAutoscaling(restAutoscalingCRWithAddress(address))
		if err == nil || !strings.Contains(err.Error(), "serverAddress") || !strings.Contains(err.Error(), "HTTPS") {
			t.Fatalf("validateAutoscaling(%q) error = %v, want serverAddress HTTPS rejection", address, err)
		}
	}
}

func TestValidateAutoscalingRejectsUntrustedHTTPSHost(t *testing.T) {
	t.Parallel()

	err := validateAutoscaling(restAutoscalingCRWithAddress("https://attacker.example"))
	if err == nil || !strings.Contains(err.Error(), "cluster monitoring endpoint") {
		t.Fatalf("validateAutoscaling() error = %v, want untrusted host rejection", err)
	}
}

func TestValidateAutoscalingRejectsMonitoringRouteURL(t *testing.T) {
	t.Parallel()

	err := validateAutoscaling(restAutoscalingCRWithAddress("https://thanos-querier-openshift-monitoring.apps.cluster.example.com"))
	if err == nil || !strings.Contains(err.Error(), "cluster monitoring endpoint") {
		t.Fatalf("validateAutoscaling() error = %v, want route URL rejection", err)
	}
}

func TestValidateAutoscalingRejectsMalformedServerAddress(t *testing.T) {
	t.Parallel()

	for _, address := range []string{
		"not a url",
		"thanos-querier.openshift-monitoring.svc.cluster.local:9092",
	} {
		err := validateAutoscaling(restAutoscalingCRWithAddress(address))
		if err == nil || !strings.Contains(err.Error(), "HTTPS") {
			t.Fatalf("validateAutoscaling(%q) error = %v, want malformed URL rejection", address, err)
		}
	}
}

func TestValidateAutoscalingRejectsWrongOrMissingPort(t *testing.T) {
	t.Parallel()

	for _, address := range []string{
		"https://thanos-querier.openshift-monitoring.svc.cluster.local:9091",
		"https://thanos-querier.openshift-monitoring.svc.cluster.local:9090",
		"https://thanos-querier.openshift-monitoring.svc.cluster.local",
	} {
		err := validateAutoscaling(restAutoscalingCRWithAddress(address))
		if err == nil || !strings.Contains(err.Error(), "cluster monitoring endpoint") {
			t.Fatalf("validateAutoscaling(%q) error = %v, want port rejection", address, err)
		}
	}
}

func TestValidateAutoscalingRejectsBadFlightServerAddress(t *testing.T) {
	t.Parallel()

	err := validateAutoscaling(flightAutoscalingCRWithAddress("http://attacker.example"))
	if err == nil || !strings.Contains(err.Error(), "flightService.autoScaling.triggers[0].serverAddress") {
		t.Fatalf("validateAutoscaling() error = %v, want flightService serverAddress rejection", err)
	}
}

func TestValidateAutoscalingNamesFailingTriggerIndex(t *testing.T) {
	t.Parallel()

	cr := restAutoscalingCRWithAddress(autoscalingMonitoringEndpoint)
	evil := validAutoscalingTrigger()
	evil.Name = "evil"
	evil.ServerAddress = "https://attacker.example"
	cr.Spec.RestService.AutoScaling.Triggers = append(cr.Spec.RestService.AutoScaling.Triggers, evil)

	err := validateAutoscaling(cr)
	if err == nil || !strings.Contains(err.Error(), "triggers[1].serverAddress") {
		t.Fatalf("validateAutoscaling() error = %v, want failing trigger index in message", err)
	}
}

func validAutoscalingConfig() *dchv1alpha1.AutoScalingConfig {
	return &dchv1alpha1.AutoScalingConfig{
		MinReplicas: ptr.To(int32(1)),
		MaxReplicas: ptr.To(int32(3)),
		Triggers:    []dchv1alpha1.AutoscalingTrigger{validAutoscalingTrigger()},
	}
}

func newReplicatedDeployment(name string) *unstructured.Unstructured {
	return &unstructured.Unstructured{Object: map[string]any{
		"apiVersion":    "apps/v1",
		"kind":          "Deployment",
		testMetadataKey: map[string]any{"name": name},
		"spec":          map[string]any{"replicas": int64(1)},
	}}
}

func TestRemoveReplicasFromAutoscaledDeployments(t *testing.T) {
	t.Parallel()

	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: autoscalingTestCRName},
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{AutoScaling: validAutoscalingConfig()},
		},
	}

	restDeploy := newReplicatedDeployment(renderedRestDeploymentName)
	flightDeploy := newReplicatedDeployment(renderedFlightDeploymentName(cr.Name))
	otherDeploy := newReplicatedDeployment("dch-unrelated")

	removeReplicasFromAutoscaledDeployments([]*unstructured.Unstructured{restDeploy, flightDeploy, otherDeploy}, cr)

	if _, found, _ := unstructured.NestedInt64(restDeploy.Object, "spec", "replicas"); found {
		t.Error("autoscaled REST deployment still carries spec.replicas")
	}
	if replicas, found, _ := unstructured.NestedInt64(flightDeploy.Object, "spec", "replicas"); !found || replicas != 1 {
		t.Errorf("non-autoscaled flight deployment replicas = (%d, found=%t), want (1, true)", replicas, found)
	}
	if replicas, found, _ := unstructured.NestedInt64(otherDeploy.Object, "spec", "replicas"); !found || replicas != 1 {
		t.Errorf("unrelated deployment replicas = (%d, found=%t), want (1, true)", replicas, found)
	}
}

func TestRemoveReplicasFromAutoscaledDeploymentsBothServices(t *testing.T) {
	t.Parallel()

	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: autoscalingTestCRName},
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{AutoScaling: validAutoscalingConfig()},
			FlightService: &dchv1alpha1.FlightServiceConfig{ServiceOverrides: dchv1alpha1.ServiceOverrides{
				AutoScaling: validAutoscalingConfig(),
			}},
		},
	}

	restDeploy := newReplicatedDeployment(renderedRestDeploymentName)
	flightDeploy := newReplicatedDeployment(renderedFlightDeploymentName(cr.Name))

	removeReplicasFromAutoscaledDeployments([]*unstructured.Unstructured{restDeploy, flightDeploy}, cr)

	for _, deploy := range []*unstructured.Unstructured{restDeploy, flightDeploy} {
		if _, found, _ := unstructured.NestedInt64(deploy.Object, "spec", "replicas"); found {
			t.Errorf("autoscaled deployment %s still carries spec.replicas", deploy.GetName())
		}
	}
}

func validAutoscalingTrigger() dchv1alpha1.AutoscalingTrigger {
	return dchv1alpha1.AutoscalingTrigger{
		Name:          "request-rate",
		ServerAddress: autoscalingMonitoringEndpoint,
		Query:         "sum(rate(requests_total[2m]))",
		Target:        dchv1alpha1.AutoscalingTarget{Type: "Value", Value: "0.4"},
	}
}

// --- Name generation: pure concatenation, no truncation ---

func autoscalingNameSuffixes() []string {
	return []string{
		autoscalingServiceAccountSuffix,
		autoscalingSecretSuffix,
		autoscalingAuthSuffix,
		autoscalingRoleSuffix,
		autoscalingBindingSuffix,
		autoscalingViewBindingSuffix,
		"-rest" + autoscalingScaledObjectSuffix,
		"-flight" + autoscalingScaledObjectSuffix,
	}
}

func TestAutoscalingNameNeverTruncates(t *testing.T) {
	t.Parallel()

	crName := strings.Repeat("a", 70)
	for _, suffix := range autoscalingNameSuffixes() {
		if got, want := autoscalingName(crName, suffix), crName+suffix; got != want {
			t.Errorf("autoscalingName(70-char name, %q) = %q, want full concatenation %q", suffix, got, want)
		}
	}
}

func TestAutoscalingNameDistinctForCRNamesSharingPrefix(t *testing.T) {
	t.Parallel()

	prefix := strings.Repeat("a", 63)
	cr1, cr2 := prefix+"-one", prefix+"-two"
	for _, suffix := range autoscalingNameSuffixes() {
		if autoscalingName(cr1, suffix) == autoscalingName(cr2, suffix) {
			t.Errorf("autoscalingName() collides for suffix %q: both CRs yield %q", suffix, autoscalingName(cr1, suffix))
		}
	}
}

func TestAutoscalingResourceNamesUniqueForLongCRName(t *testing.T) {
	t.Parallel()

	crName := strings.Repeat("a", 63) + "-one"
	resources := autoscalingResourceNames(crName)
	if len(resources) != 8 {
		t.Fatalf("autoscalingResourceNames() rendered %d resources, want 8", len(resources))
	}

	seen := map[string]struct{}{}
	for _, resource := range resources {
		key := resource.GetKind() + "/" + resource.GetName()
		if _, duplicate := seen[key]; duplicate {
			t.Errorf("autoscalingResourceNames() yields duplicate %s", key)
		}
		seen[key] = struct{}{}
		if !strings.HasPrefix(resource.GetName(), crName) {
			t.Errorf("name %q does not preserve the full CR name", resource.GetName())
		}
	}
}

// --- Reconcile-level behavior with a fake client ---

const autoscalingTestNamespace = "dch-autoscaling-test"

func autoscalingTestScheme(t *testing.T) *runtime.Scheme {
	t.Helper()
	scheme := runtime.NewScheme()
	if err := dchv1alpha1.AddToScheme(scheme); err != nil {
		t.Fatalf("registering DataConnectService scheme: %v", err)
	}
	for _, gvk := range []schema.GroupVersionKind{
		{Version: "v1", Kind: kindServiceAccount},
		{Version: "v1", Kind: kindSecret},
		{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRole},
		{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRoleBinding},
		scaledObjectGVK,
		triggerAuthGVK,
	} {
		scheme.AddKnownTypeWithName(gvk, &unstructured.Unstructured{})
		scheme.AddKnownTypeWithName(gvk.GroupVersion().WithKind(gvk.Kind+"List"), &unstructured.UnstructuredList{})
	}
	return scheme
}

func autoscalingEnabledCR(name, uid string) *dchv1alpha1.DataConnectService {
	return &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: autoscalingTestNamespace, UID: types.UID(uid)},
		Spec: dchv1alpha1.DataConnectServiceSpec{
			RestService: &dchv1alpha1.ServiceOverrides{AutoScaling: validAutoscalingConfig()},
			FlightService: &dchv1alpha1.FlightServiceConfig{ServiceOverrides: dchv1alpha1.ServiceOverrides{
				AutoScaling: validAutoscalingConfig(),
			}},
		},
	}
}

// autoscalingFakeBuilder returns a fake client builder backed by a plain
// object tracker. The default field-managed tracker cannot compare
// unstructured objects registered under several GVKs; a plain tracker is
// sufficient because applyResources never relies on managed fields.
func autoscalingFakeBuilder(t *testing.T, scheme *runtime.Scheme) *fake.ClientBuilder {
	t.Helper()
	tracker := clientgotesting.NewObjectTracker(scheme, serializer.NewCodecFactory(scheme).UniversalDecoder())
	return fake.NewClientBuilder().WithScheme(scheme).WithObjectTracker(tracker)
}

func newAutoscalingReconciler(t *testing.T, objects ...client.Object) (*DataConnectServiceReconciler, client.Client) {
	t.Helper()
	scheme := autoscalingTestScheme(t)
	cli := autoscalingFakeBuilder(t, scheme).WithObjects(objects...).Build()
	return &DataConnectServiceReconciler{Client: cli, Scheme: scheme}, cli
}

func getAutoscalingResource(t *testing.T, cli client.Client, resource *unstructured.Unstructured) *unstructured.Unstructured {
	t.Helper()
	live := &unstructured.Unstructured{}
	live.SetGroupVersionKind(resource.GroupVersionKind())
	key := client.ObjectKey{Namespace: autoscalingTestNamespace, Name: resource.GetName()}
	if err := cli.Get(context.Background(), key, live); err != nil {
		t.Fatalf("getting %s %s: %v", resource.GetKind(), resource.GetName(), err)
	}
	return live
}

func TestReconcileAutoscalingTwoLongNameCRsUniqueAndOwned(t *testing.T) {
	t.Parallel()

	prefix := strings.Repeat("a", 63)
	cr1 := autoscalingEnabledCR(prefix+"-one", "uid-cr-one")
	cr2 := autoscalingEnabledCR(prefix+"-two", "uid-cr-two")
	r, cli := newAutoscalingReconciler(t)
	ctx := context.Background()

	for _, cr := range []*dchv1alpha1.DataConnectService{cr1, cr2} {
		if err := r.reconcileAutoscaling(ctx, cr); err != nil {
			t.Fatalf("reconcileAutoscaling(%s) failed: %v", cr.Name, err)
		}
	}

	seen := map[string]string{}
	for _, cr := range []*dchv1alpha1.DataConnectService{cr1, cr2} {
		for _, resource := range autoscalingResourceNames(cr.Name) {
			key := resource.GetKind() + "/" + resource.GetName()
			if other, duplicate := seen[key]; duplicate {
				t.Errorf("%s is generated for both CR %s and CR %s", key, other, cr.Name)
			}
			seen[key] = cr.Name

			live := getAutoscalingResource(t, cli, resource)
			if !hasControllerOwner(live, cr.UID) {
				t.Errorf("%s is not owned by CR %s (uid %s)", key, cr.Name, cr.UID)
			}
		}
	}
}

func TestReconcileAutoscalingStableAcrossReconciles(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-stable")
	r, cli := newAutoscalingReconciler(t)
	ctx := context.Background()

	if err := r.reconcileAutoscaling(ctx, cr); err != nil {
		t.Fatalf("first reconcileAutoscaling() failed: %v", err)
	}

	versions := map[string]string{}
	for _, resource := range autoscalingResourceNames(cr.Name) {
		live := getAutoscalingResource(t, cli, resource)
		versions[resource.GetKind()+"/"+resource.GetName()] = live.GetResourceVersion()
	}

	if err := r.reconcileAutoscaling(ctx, cr); err != nil {
		t.Fatalf("second reconcileAutoscaling() failed: %v", err)
	}

	for _, resource := range autoscalingResourceNames(cr.Name) {
		key := resource.GetKind() + "/" + resource.GetName()
		live := getAutoscalingResource(t, cli, resource)
		if live.GetResourceVersion() != versions[key] {
			t.Errorf("%s was rewritten by a no-op reconcile (resourceVersion %q -> %q)", key, versions[key], live.GetResourceVersion())
		}
	}
}

func TestPruneAutoscalingResourcesRemovesOnlyOwnedResources(t *testing.T) {
	t.Parallel()

	prefix := strings.Repeat("a", 63)
	cr1 := autoscalingEnabledCR(prefix+"-one", "uid-prune-one")
	cr2 := autoscalingEnabledCR(prefix+"-two", "uid-prune-two")
	r, cli := newAutoscalingReconciler(t)
	ctx := context.Background()

	for _, cr := range []*dchv1alpha1.DataConnectService{cr1, cr2} {
		if err := r.reconcileAutoscaling(ctx, cr); err != nil {
			t.Fatalf("reconcileAutoscaling(%s) failed: %v", cr.Name, err)
		}
	}

	// Disabling autoscaling on CR2 must prune exactly CR2's resources.
	if err := r.pruneAutoscalingResources(ctx, cr2, nil); err != nil {
		t.Fatalf("pruneAutoscalingResources() failed: %v", err)
	}

	for _, resource := range autoscalingResourceNames(cr2.Name) {
		live := &unstructured.Unstructured{}
		live.SetGroupVersionKind(resource.GroupVersionKind())
		key := client.ObjectKey{Namespace: autoscalingTestNamespace, Name: resource.GetName()}
		err := cli.Get(ctx, key, live)
		if !apierrors.IsNotFound(err) {
			t.Errorf("CR2 resource %s %s was not pruned (err = %v)", resource.GetKind(), resource.GetName(), err)
		}
	}
	for _, resource := range autoscalingResourceNames(cr1.Name) {
		live := getAutoscalingResource(t, cli, resource)
		if !hasControllerOwner(live, cr1.UID) {
			t.Errorf("CR1 resource %s %s lost its owner during CR2 prune", resource.GetKind(), resource.GetName())
		}
	}
}

// --- applyResources: admission gate for pre-existing resources ---

// seedForApply mirrors the metadata applyResources stamps on created objects
// (namespace, managed-by label, spec-hash annotation) so a seeded object
// matches the desired content. Owner references and annotations are excluded
// from the content hash, so callers may set either afterwards.
func seedForApply(obj *unstructured.Unstructured) *unstructured.Unstructured {
	seed := obj.DeepCopy()
	seed.SetNamespace(autoscalingTestNamespace)
	labels := seed.GetLabels()
	if labels == nil {
		labels = map[string]string{}
	}
	labels[labelManagedBy] = managedByDCHService
	seed.SetLabels(labels)
	annotations := seed.GetAnnotations()
	if annotations == nil {
		annotations = map[string]string{}
	}
	annotations[annotationSpecHash] = specHash(seed)
	seed.SetAnnotations(annotations)
	return seed
}

func controllerOwnerRef(cr *dchv1alpha1.DataConnectService) metav1.OwnerReference {
	return metav1.OwnerReference{
		APIVersion: dchv1alpha1.GroupVersion.String(),
		Kind:       kindDataConnectService,
		Name:       cr.Name,
		UID:        cr.UID,
		Controller: ptr.To(true),
	}
}

func foreignControllerOwnerRef() metav1.OwnerReference {
	return metav1.OwnerReference{
		APIVersion: dchv1alpha1.GroupVersion.String(),
		Kind:       kindDataConnectService,
		Name:       "other-dataconnectservice",
		UID:        "uid-foreign",
		Controller: ptr.To(true),
	}
}

// staleControllerOwnerRef mimics a dependent left behind by a previous
// incarnation of the CR: same group, kind, and name, different UID.
func staleControllerOwnerRef(cr *dchv1alpha1.DataConnectService) metav1.OwnerReference {
	ref := controllerOwnerRef(cr)
	ref.UID = types.UID("uid-previous-incarnation")
	return ref
}

func autoscalingServiceAccountOf(t *testing.T, cr *dchv1alpha1.DataConnectService) *unstructured.Unstructured {
	t.Helper()
	for _, resource := range autoscalingResources(cr) {
		if resource.GetKind() == kindServiceAccount {
			return resource
		}
	}
	t.Fatal("no ServiceAccount in rendered autoscaling resources")
	return nil
}

func autoscalingSecretOf(t *testing.T, cr *dchv1alpha1.DataConnectService) *unstructured.Unstructured {
	t.Helper()
	for _, resource := range autoscalingResources(cr) {
		if resource.GetKind() == kindSecret {
			return resource
		}
	}
	t.Fatal("no Secret in rendered autoscaling resources")
	return nil
}

// requireResourceConflict asserts that applyResources rejected the given
// resource with a ResourceConflictError naming it.
func requireResourceConflict(t *testing.T, err error, resource *unstructured.Unstructured) {
	t.Helper()
	if err == nil {
		t.Fatalf("applyResources() = nil error, want a resource conflict for %s %s", resource.GetKind(), resource.GetName())
	}
	var conflict *ResourceConflictError
	if !errors.As(err, &conflict) {
		t.Fatalf("applyResources() error = %v, want ResourceConflictError", err)
	}
	if conflict.Kind != resource.GetKind() || conflict.Name != resource.GetName() {
		t.Fatalf("applyResources() conflict reports %s %q, want %s %q",
			conflict.Kind, conflict.Name, resource.GetKind(), resource.GetName())
	}
}

// addForeignLabel mutates the seed so its content differs from the rendered
// desired state (labels are part of the spec hash).
func addForeignLabel(obj *unstructured.Unstructured) {
	labels := obj.GetLabels()
	if labels == nil {
		labels = map[string]string{}
	}
	labels["conflict.example/foreign"] = "true"
	obj.SetLabels(labels)
}

// TestApplyResourcesRejectsForeignOwnedResources covers every generated
// autoscaling resource pre-created with a foreign controller owner and an
// otherwise matching specification: reconciliation must reject the object
// instead of adopting or overwriting it.
func TestApplyResourcesRejectsForeignOwnedResources(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-foreign")
	for _, resource := range autoscalingResources(cr) {
		t.Run(resource.GetKind()+"/"+resource.GetName(), func(t *testing.T) {
			t.Parallel()

			seed := seedForApply(resource)
			seed.SetOwnerReferences([]metav1.OwnerReference{foreignControllerOwnerRef()})

			r, _ := newAutoscalingReconciler(t, seed)
			err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{resource.DeepCopy()})
			requireResourceConflict(t, err, resource)
		})
	}
}

// TestApplyResourcesRejectsForeignOwnedResourcesWithDifferingSpec is the
// combined case from the security review: conflicting ownership and a
// conflicting specification at once. Ownership is verified before content, so
// the object is rejected without ever being force-applied.
func TestApplyResourcesRejectsForeignOwnedResourcesWithDifferingSpec(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-foreign-spec")
	for _, resource := range autoscalingResources(cr) {
		t.Run(resource.GetKind()+"/"+resource.GetName(), func(t *testing.T) {
			t.Parallel()

			seed := seedForApply(resource)
			addForeignLabel(seed)
			seed.SetOwnerReferences([]metav1.OwnerReference{foreignControllerOwnerRef()})

			r, _ := newAutoscalingReconciler(t, seed)
			err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{resource.DeepCopy()})
			requireResourceConflict(t, err, resource)
		})
	}
}

// TestApplyResourcesRejectsUnownedResourcesWithConflictingSpec covers every
// generated autoscaling resource pre-created without any owner reference but
// with content differing from the desired state — both an object the
// controller never managed and one that drifted from a managed state.
// Reconciliation must reject the object rather than silently overwriting or
// trusting it.
func TestApplyResourcesRejectsUnownedResourcesWithConflictingSpec(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-spec")
	for _, resource := range autoscalingResources(cr) {
		for _, seedKind := range []string{"unmanaged", "drifted"} {
			t.Run(resource.GetKind()+"/"+resource.GetName()+"/"+seedKind, func(t *testing.T) {
				t.Parallel()

				var seed *unstructured.Unstructured
				if seedKind == "unmanaged" {
					seed = resource.DeepCopy()
					seed.SetNamespace(autoscalingTestNamespace)
				} else {
					seed = seedForApply(resource)
				}
				addForeignLabel(seed)

				r, _ := newAutoscalingReconciler(t, seed)
				err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{resource.DeepCopy()})
				requireResourceConflict(t, err, resource)
			})
		}
	}
}

// TestApplyResourcesRejectsConflictingTokenSecret covers the failing scenario
// from the security review: a pre-existing Secret with the deterministic
// autoscaling token name containing attacker-controlled data, a wrong type, a
// wrong service-account annotation, or a copied spec-hash annotation. KEDA's
// TriggerAuthentication consumes this Secret as a bearer token, so
// reconciliation must reject it rather than proceeding.
func TestApplyResourcesRejectsConflictingTokenSecret(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-secret")
	secret := autoscalingSecretOf(t, cr)

	cases := []struct {
		name    string
		mutate  func(*unstructured.Unstructured)
		wantMsg string
	}{
		{
			name: "wrong type",
			mutate: func(s *unstructured.Unstructured) {
				s.Object["type"] = secretTypeOpaque
			},
			wantMsg: "type is",
		},
		{
			name: "wrong service account annotation",
			mutate: func(s *unstructured.Unstructured) {
				annotations := s.GetAnnotations()
				annotations["kubernetes.io/service-account.name"] = "attacker-sa"
				s.SetAnnotations(annotations)
			},
			wantMsg: "annotation kubernetes.io/service-account.name",
		},
		{
			name: "attacker-controlled token data",
			mutate: func(s *unstructured.Unstructured) {
				s.Object["data"] = map[string]any{autoscalingTokenKey: attackerTokenValue}
			},
			wantMsg: "unexpected data",
		},
		{
			name: "foreign data with a copied spec-hash annotation",
			mutate: func(s *unstructured.Unstructured) {
				// Copy a plausible spec-hash annotation onto the foreign
				// Secret: the hash-equality fast path must not trust the
				// annotation without verifying the content.
				annotations := s.GetAnnotations()
				if annotations == nil {
					annotations = map[string]string{}
				}
				annotations[annotationSpecHash] = specHash(s)
				s.SetAnnotations(annotations)
				s.Object["data"] = map[string]any{autoscalingTokenKey: attackerTokenValue}
			},
			wantMsg: "unexpected data",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			seed := secret.DeepCopy()
			seed.SetNamespace(autoscalingTestNamespace)
			tc.mutate(seed)

			r, cli := newAutoscalingReconciler(t, seed)
			// desired must be a per-subtest copy: applyResources stamps
			// lineage markers on the objects it is given.
			err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{secret.DeepCopy()})
			requireResourceConflict(t, err, secret)
			if !strings.Contains(err.Error(), tc.wantMsg) {
				t.Fatalf("applyResources() error = %v, want it to mention %q", err, tc.wantMsg)
			}

			// The rejected Secret must be left untouched: no lineage
			// markers are stamped and the foreign data survives intact.
			live := getAutoscalingResource(t, cli, seed)
			if len(live.GetOwnerReferences()) != 0 {
				t.Error("rejected Secret was stamped with an owner reference")
			}
			if live.GetLabels()[labelManagedBy] != "" {
				t.Error("rejected Secret was stamped with the managed-by label")
			}
			if token, _, _ := unstructured.NestedString(live.Object, "data", autoscalingTokenKey); seed.Object["data"] != nil && token != attackerTokenValue {
				t.Errorf("rejected Secret data was modified: token = %q", token)
			}
		})
	}
}

// TestApplyResourcesAdoptsExactMatchUnownedResources covers the benign
// counterpart: an unowned object whose content exactly matches the desired
// state — including the managed-by label the controller stamps on everything
// it manages — is adopted, not rejected.
func TestApplyResourcesAdoptsExactMatchUnownedResources(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-adopt")
	for _, resource := range autoscalingResources(cr) {
		t.Run(resource.GetKind()+"/"+resource.GetName(), func(t *testing.T) {
			t.Parallel()

			seed := seedForApply(resource)

			r, cli := newAutoscalingReconciler(t, seed)
			if err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{resource.DeepCopy()}); err != nil {
				t.Fatalf("applyResources() failed on an exact-match unowned resource: %v", err)
			}

			live := getAutoscalingResource(t, cli, resource)
			if !hasControllerOwner(live, cr.UID) {
				t.Errorf("adopted %s %s has no controller owner reference", resource.GetKind(), resource.GetName())
			}
			if live.GetAnnotations()[annotationSpecHash] == "" {
				t.Errorf("adopted %s %s has no spec-hash annotation", resource.GetKind(), resource.GetName())
			}
		})
	}
}

// TestApplyResourcesReclaimsStaleIncarnationResources covers a CR that was
// deleted and recreated before garbage collection removed its old dependents:
// the stale controller reference names the same CR, so matching objects are
// reclaimed and the stale reference is replaced with the current one.
func TestApplyResourcesReclaimsStaleIncarnationResources(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-stale")
	for _, resource := range autoscalingResources(cr) {
		t.Run(resource.GetKind()+"/"+resource.GetName(), func(t *testing.T) {
			t.Parallel()

			seed := seedForApply(resource)
			seed.SetOwnerReferences([]metav1.OwnerReference{staleControllerOwnerRef(cr)})

			r, cli := newAutoscalingReconciler(t, seed)
			if err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{resource.DeepCopy()}); err != nil {
				t.Fatalf("applyResources() failed to reclaim a stale-incarnation resource: %v", err)
			}

			live := getAutoscalingResource(t, cli, resource)
			if !hasControllerOwner(live, cr.UID) {
				t.Errorf("reclaimed %s %s still carries the stale owner reference", resource.GetKind(), resource.GetName())
			}
		})
	}
}

// TestApplyResourcesRejectsDriftedStaleIncarnationResource ensures the stale
// incarnation tolerance is not a free pass: a stale reference plus drifted
// content is still rejected, because the controller cannot know whether the
// drift is benign.
func TestApplyResourcesRejectsDriftedStaleIncarnationResource(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-stale-drift")
	sa := autoscalingServiceAccountOf(t, cr)

	seed := seedForApply(sa)
	addForeignLabel(seed)
	seed.SetOwnerReferences([]metav1.OwnerReference{staleControllerOwnerRef(cr)})

	r, _ := newAutoscalingReconciler(t, seed)
	err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{sa})
	requireResourceConflict(t, err, sa)
}

// TestApplyResourcesKeepsInjectedTokenOnTrustedSecret ensures a Secret the
// controller already owns is never flagged when Kubernetes has injected token
// data into it: the injected token is expected server-side state, not drift.
func TestApplyResourcesKeepsInjectedTokenOnTrustedSecret(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-trusted-secret")
	secret := autoscalingSecretOf(t, cr)

	seed := seedForApply(secret)
	seed.SetOwnerReferences([]metav1.OwnerReference{controllerOwnerRef(cr)})
	seed.Object["data"] = map[string]any{
		"token":     "injected-service-account-token",
		"ca.crt":    "ca-bundle",
		"namespace": "dch-autoscaling-test",
	}

	r, cli := newAutoscalingReconciler(t, seed)
	if err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{secret}); err != nil {
		t.Fatalf("applyResources() rejected a trusted Secret with injected token data: %v", err)
	}

	live := getAutoscalingResource(t, cli, seed)
	if token, _, _ := unstructured.NestedString(live.Object, "data", "token"); token != "injected-service-account-token" {
		t.Errorf("injected token data was modified: token = %q", token)
	}
	if !hasControllerOwner(live, cr.UID) {
		t.Error("trusted Secret lost its controller owner reference")
	}
}

// TestApplyResourcesRepairsDriftedOwnedResource ensures an object the
// controller owns is repaired via apply when its content drifted, without the
// admission gate rejecting it. This uses the default field-managed fake
// client because the plain tracker used elsewhere cannot serve apply patches.
func TestApplyResourcesRepairsDriftedOwnedResource(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-gate-drift")
	sa := autoscalingServiceAccountOf(t, cr)

	seed := sa.DeepCopy()
	seed.SetNamespace(autoscalingTestNamespace)
	seed.SetOwnerReferences([]metav1.OwnerReference{controllerOwnerRef(cr)})
	addForeignLabel(seed)

	scheme := autoscalingTestScheme(t)
	cli := fake.NewClientBuilder().WithScheme(scheme).WithObjects(seed).Build()
	r := &DataConnectServiceReconciler{Client: cli, Scheme: scheme}

	if err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{sa}); err != nil {
		t.Fatalf("applyResources() failed to repair a drifted owned resource: %v", err)
	}

	live := getAutoscalingResource(t, cli, seed)
	if !hasControllerOwner(live, cr.UID) {
		t.Error("repaired ServiceAccount lost its controller owner reference")
	}
	if live.GetLabels()[labelManagedBy] != managedByDCHService {
		t.Error("repaired ServiceAccount has no managed-by label")
	}
	if live.GetAnnotations()[annotationSpecHash] == "" {
		t.Error("repaired ServiceAccount has no spec-hash annotation")
	}
}

func TestApplyResourcesSurfacesForeignOwnedResource(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-foreign-test")
	sa := autoscalingServiceAccountOf(t, cr)

	seed := seedForApply(sa)
	seed.SetOwnerReferences([]metav1.OwnerReference{foreignControllerOwnerRef()})

	r, _ := newAutoscalingReconciler(t, seed)
	err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{sa})
	requireResourceConflict(t, err, sa)
}

func TestApplyResourcesCreateRaceSurfacesForeignOwner(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-race-test")
	sa := autoscalingServiceAccountOf(t, cr)

	seed := seedForApply(sa)
	seed.SetOwnerReferences([]metav1.OwnerReference{foreignControllerOwnerRef()})

	scheme := autoscalingTestScheme(t)
	getCalls := 0
	cli := autoscalingFakeBuilder(t, scheme).
		WithObjects(seed).
		WithInterceptorFuncs(interceptor.Funcs{
			// Hide the seeded object from the initial read so the create
			// path runs and loses the race with the existing object.
			Get: func(ctx context.Context, c client.WithWatch, key client.ObjectKey, obj client.Object, opts ...client.GetOption) error {
				if getCalls == 0 {
					getCalls++
					return apierrors.NewNotFound(schema.GroupResource{Resource: "serviceaccounts"}, key.Name)
				}
				return c.Get(ctx, key, obj, opts...)
			},
		}).
		Build()
	r := &DataConnectServiceReconciler{Client: cli, Scheme: scheme}

	err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{sa})
	requireResourceConflict(t, err, sa)
}

func TestApplyResourcesCreateRaceWithOwnResourceSucceeds(t *testing.T) {
	t.Parallel()

	cr := autoscalingEnabledCR(autoscalingTestCRName, "uid-race-own-test")
	sa := autoscalingServiceAccountOf(t, cr)

	seed := seedForApply(sa)
	seed.SetOwnerReferences([]metav1.OwnerReference{controllerOwnerRef(cr)})

	scheme := autoscalingTestScheme(t)
	getCalls := 0
	cli := autoscalingFakeBuilder(t, scheme).
		WithObjects(seed).
		WithInterceptorFuncs(interceptor.Funcs{
			Get: func(ctx context.Context, c client.WithWatch, key client.ObjectKey, obj client.Object, opts ...client.GetOption) error {
				if getCalls == 0 {
					getCalls++
					return apierrors.NewNotFound(schema.GroupResource{Resource: "serviceaccounts"}, key.Name)
				}
				return c.Get(ctx, key, obj, opts...)
			},
		}).
		Build()
	r := &DataConnectServiceReconciler{Client: cli, Scheme: scheme}

	if err := r.applyResources(context.Background(), cr, autoscalingTestNamespace, []*unstructured.Unstructured{sa}); err != nil {
		t.Fatalf("applyResources() failed on a benign create race: %v", err)
	}

	live := &unstructured.Unstructured{}
	live.SetGroupVersionKind(sa.GroupVersionKind())
	key := client.ObjectKey{Namespace: autoscalingTestNamespace, Name: sa.GetName()}
	if err := cli.Get(context.Background(), key, live); err != nil {
		t.Fatalf("getting ServiceAccount after create race: %v", err)
	}
	if !hasControllerOwner(live, cr.UID) {
		t.Errorf("ServiceAccount lost its owner during the create race")
	}
}
