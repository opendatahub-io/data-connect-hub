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
	"fmt"
	"maps"
	"strings"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

var errAutoscalingProviderUnavailable = errors.New("CMA/KEDA API is unavailable")

var (
	scaledObjectGVK = schema.GroupVersionKind{Group: "keda.sh", Version: "v1alpha1", Kind: "ScaledObject"}
	triggerAuthGVK  = schema.GroupVersionKind{Group: "keda.sh", Version: "v1alpha1", Kind: "TriggerAuthentication"}
)

const (
	autoscalingServiceAccountSuffix = "-autoscaling-sa"
	autoscalingSecretSuffix         = "-autoscaling-token"
	autoscalingAuthSuffix           = "-autoscaling-auth"
	autoscalingRoleSuffix           = "-autoscaling-metrics-reader"
	autoscalingBindingSuffix        = "-autoscaling-metrics-reader"
	autoscalingViewBindingSuffix    = "-autoscaling-thanos-view"
	autoscalingScaledObjectSuffix   = "-autoscaling"
	autoscalingMetadataField        = "metadata"
	autoscalingSpecField            = "spec"
	autoscalingNameField            = "name"
	autoscalingKindField            = "kind"
	autoscalingRBACGroup            = "rbac.authorization.k8s.io"
)

type autoscalingService struct {
	name       string
	deployment string
	replicas   *int32
	config     *dchv1alpha1.AutoScalingConfig
}

func validateAutoscaling(cr *dchv1alpha1.DataConnectService) error {
	services := []autoscalingService{
		{name: "restService", deployment: renderedRestDeploymentName, replicas: serviceReplicas(cr.Spec.RestService), config: serviceAutoScaling(cr.Spec.RestService)},
		{name: "flightService", deployment: renderedFlightDeploymentName(cr.Name), replicas: flightReplicas(cr.Spec.FlightService), config: flightAutoScaling(cr.Spec.FlightService)},
	}

	for _, service := range services {
		if service.config == nil {
			continue
		}
		if service.replicas != nil {
			return fmt.Errorf("%s.replicas cannot be used with autoScaling", service.name)
		}
		if service.config.MinReplicas == nil || service.config.MaxReplicas == nil {
			return fmt.Errorf("%s.autoScaling requires minReplicas and maxReplicas", service.name)
		}
		if *service.config.MinReplicas > *service.config.MaxReplicas {
			return fmt.Errorf("%s.autoScaling minReplicas cannot exceed maxReplicas", service.name)
		}
		if len(service.config.Triggers) == 0 {
			return fmt.Errorf("%s.autoScaling requires at least one trigger", service.name)
		}
		for i, trigger := range service.config.Triggers {
			if strings.TrimSpace(trigger.Name) == "" {
				return fmt.Errorf("%s.autoScaling.triggers[%d].name is required", service.name, i)
			}
			if strings.TrimSpace(trigger.ServerAddress) == "" {
				return fmt.Errorf("%s.autoScaling.triggers[%d].serverAddress is required", service.name, i)
			}
			if strings.TrimSpace(trigger.Query) == "" {
				return fmt.Errorf("%s.autoScaling.triggers[%d].query is required", service.name, i)
			}
			if trigger.Target.Type != "Value" {
				return fmt.Errorf("%s.autoScaling.triggers[%d].target.type must be Value", service.name, i)
			}
			if strings.TrimSpace(trigger.Target.Value) == "" {
				return fmt.Errorf("%s.autoScaling.triggers[%d].target.value is required", service.name, i)
			}
		}
	}
	return nil
}

func serviceAutoScaling(overrides *dchv1alpha1.ServiceOverrides) *dchv1alpha1.AutoScalingConfig {
	if overrides == nil {
		return nil
	}
	return overrides.AutoScaling
}

func serviceReplicas(overrides *dchv1alpha1.ServiceOverrides) *int32 {
	if overrides == nil {
		return nil
	}
	return overrides.Replicas
}

func flightAutoScaling(config *dchv1alpha1.FlightServiceConfig) *dchv1alpha1.AutoScalingConfig {
	if config == nil {
		return nil
	}
	return config.AutoScaling
}

func flightReplicas(config *dchv1alpha1.FlightServiceConfig) *int32 {
	if config == nil {
		return nil
	}
	return config.Replicas
}

const renderedRestDeploymentName = "dch-rest-service"

func renderedFlightDeploymentName(crName string) string {
	return "dch-" + flightServiceResourceName(crName)
}

// removeReplicasFromAutoscaledDeployments drops spec.replicas from
// deployments managed by KEDA. The base manifests pin replicas to 1, and a
// later force-apply of that field would reset the KEDA-managed replica count.
func removeReplicasFromAutoscaledDeployments(resources []*unstructured.Unstructured, cr *dchv1alpha1.DataConnectService) {
	autoscaled := map[string]struct{}{}
	if serviceAutoScaling(cr.Spec.RestService) != nil {
		autoscaled[renderedRestDeploymentName] = struct{}{}
	}
	if flightAutoScaling(cr.Spec.FlightService) != nil {
		autoscaled[renderedFlightDeploymentName(cr.Name)] = struct{}{}
	}
	if len(autoscaled) == 0 {
		return
	}
	for _, obj := range resources {
		if obj.GetKind() != kindDeployment {
			continue
		}
		if _, ok := autoscaled[obj.GetName()]; !ok {
			continue
		}
		unstructured.RemoveNestedField(obj.Object, autoscalingSpecField, "replicas")
	}
}

func autoscalingResources(cr *dchv1alpha1.DataConnectService) []*unstructured.Unstructured {
	services := []autoscalingService{
		{name: "rest", deployment: renderedRestDeploymentName, config: serviceAutoScaling(cr.Spec.RestService)},
		{name: "flight", deployment: renderedFlightDeploymentName(cr.Name), config: flightAutoScaling(cr.Spec.FlightService)},
	}
	var configured []autoscalingService
	for _, service := range services {
		if service.config != nil {
			configured = append(configured, service)
		}
	}
	if len(configured) == 0 {
		return nil
	}

	saName := autoscalingName(cr.Name, autoscalingServiceAccountSuffix)
	secretName := autoscalingName(cr.Name, autoscalingSecretSuffix)
	authName := autoscalingName(cr.Name, autoscalingAuthSuffix)
	roleName := autoscalingName(cr.Name, autoscalingRoleSuffix)
	bindingName := autoscalingName(cr.Name, autoscalingBindingSuffix)
	viewBindingName := autoscalingName(cr.Name, autoscalingViewBindingSuffix)

	resources := []*unstructured.Unstructured{
		newAutoscalingObject("v1", kindServiceAccount, saName, schema.GroupVersionKind{Version: "v1", Kind: kindServiceAccount}, map[string]any{}),
		newAutoscalingObject("v1", kindSecret, secretName, schema.GroupVersionKind{Version: "v1", Kind: kindSecret}, map[string]any{
			"type": "kubernetes.io/service-account-token",
			autoscalingMetadataField: map[string]any{
				"annotations": map[string]any{"kubernetes.io/service-account.name": saName},
			},
		}),
		newAutoscalingObject("keda.sh/v1alpha1", "TriggerAuthentication", authName, triggerAuthGVK, map[string]any{
			autoscalingSpecField: map[string]any{
				"secretTargetRef": []any{map[string]any{
					"parameter":          "bearerToken",
					autoscalingNameField: secretName,
					"key":                "token",
				}},
			},
		}),
		newAutoscalingObject(autoscalingRBACGroup+"/v1", kindRole, roleName, schema.GroupVersionKind{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRole}, map[string]any{
			"rules": []any{
				map[string]any{"apiGroups": []any{""}, "resources": []any{"pods"}, "verbs": []any{"get"}},
				map[string]any{"apiGroups": []any{"metrics.k8s.io"}, "resources": []any{"pods"}, "verbs": []any{"get", "list", "watch"}},
			},
		}),
		newAutoscalingObject(autoscalingRBACGroup+"/v1", kindRoleBinding, bindingName, schema.GroupVersionKind{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRoleBinding}, map[string]any{
			"roleRef":  map[string]any{"apiGroup": autoscalingRBACGroup, autoscalingKindField: kindRole, autoscalingNameField: roleName},
			"subjects": []any{map[string]any{autoscalingKindField: kindServiceAccount, autoscalingNameField: saName}},
		}),
		newAutoscalingObject(autoscalingRBACGroup+"/v1", kindRoleBinding, viewBindingName, schema.GroupVersionKind{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRoleBinding}, map[string]any{
			"roleRef":  map[string]any{"apiGroup": autoscalingRBACGroup, autoscalingKindField: "ClusterRole", autoscalingNameField: "view"},
			"subjects": []any{map[string]any{autoscalingKindField: kindServiceAccount, autoscalingNameField: saName}},
		}),
	}

	for _, service := range configured {
		resources = append(resources, newScaledObject(cr.Name, cr.Namespace, service, authName))
	}
	return resources
}

func newScaledObject(crName, namespace string, service autoscalingService, authName string) *unstructured.Unstructured {
	triggers := make([]any, 0, len(service.config.Triggers))
	for _, trigger := range service.config.Triggers {
		triggers = append(triggers, map[string]any{
			"type": "prometheus",
			autoscalingMetadataField: map[string]any{
				"metricName":    trigger.Name,
				"serverAddress": trigger.ServerAddress,
				"namespace":     namespace,
				"query":         trigger.Query,
				"threshold":     trigger.Target.Value,
				"authModes":     "bearer",
			},
			"authenticationRef": map[string]any{
				autoscalingNameField: authName,
				autoscalingKindField: "TriggerAuthentication",
			},
		})
	}

	minReplicas := int64(*service.config.MinReplicas)
	maxReplicas := int64(*service.config.MaxReplicas)
	pollingInterval := int64(15)
	if service.config.PollingInterval != nil {
		pollingInterval = int64(*service.config.PollingInterval)
	}
	cooldownPeriod := int64(300)
	if service.config.CooldownPeriod != nil {
		cooldownPeriod = int64(*service.config.CooldownPeriod)
	}

	return newAutoscalingObject("keda.sh/v1alpha1", "ScaledObject", autoscalingName(crName, "-"+service.name+autoscalingScaledObjectSuffix), scaledObjectGVK, map[string]any{
		autoscalingSpecField: map[string]any{
			"scaleTargetRef": map[string]any{
				autoscalingNameField: service.deployment,
				autoscalingKindField: "Deployment",
			},
			"minReplicaCount": minReplicas,
			"maxReplicaCount": maxReplicas,
			"pollingInterval": pollingInterval,
			"cooldownPeriod":  cooldownPeriod,
			"triggers":        triggers,
		},
	})
}

func newAutoscalingObject(apiVersion, kind, name string, gvk schema.GroupVersionKind, fields map[string]any) *unstructured.Unstructured {
	object := map[string]any{
		"apiVersion":             apiVersion,
		autoscalingKindField:     kind,
		autoscalingMetadataField: map[string]any{autoscalingNameField: name},
	}
	metadata := object[autoscalingMetadataField].(map[string]any)
	for key, value := range fields {
		if key == autoscalingMetadataField {
			maps.Copy(metadata, value.(map[string]any))
			continue
		}
		object[key] = value
	}
	result := &unstructured.Unstructured{Object: object}
	if gvk != (schema.GroupVersionKind{}) {
		result.SetGroupVersionKind(gvk)
	}
	return result
}

func autoscalingName(crName, suffix string) string {
	name := crName + suffix
	if len(name) > 63 {
		name = strings.TrimSuffix(name[:63], "-")
	}
	return name
}

func (r *DataConnectServiceReconciler) reconcileAutoscaling(ctx context.Context, cr *dchv1alpha1.DataConnectService) error {
	resources := autoscalingResources(cr)
	if len(resources) > 0 {
		if err := r.requireAutoscalingProvider(ctx, cr.Namespace); err != nil {
			return err
		}
		if err := r.applyResources(ctx, cr, cr.Namespace, resources); err != nil {
			return err
		}
	}
	return r.pruneAutoscalingResources(ctx, cr, resources)
}

func (r *DataConnectServiceReconciler) requireAutoscalingProvider(ctx context.Context, namespace string) error {
	for _, gvk := range []schema.GroupVersionKind{scaledObjectGVK, triggerAuthGVK} {
		probe := &unstructured.Unstructured{}
		probe.SetGroupVersionKind(gvk)
		err := r.Get(ctx, types.NamespacedName{Name: "__dch-autoscaling-provider-probe__", Namespace: namespace}, probe)
		if err == nil || apierrors.IsNotFound(err) {
			continue
		}
		if meta.IsNoMatchError(err) {
			return fmt.Errorf("%w: %s", errAutoscalingProviderUnavailable, gvk.Kind)
		}
		return fmt.Errorf("checking %s API: %w", gvk.Kind, err)
	}
	return nil
}

func (r *DataConnectServiceReconciler) pruneAutoscalingResources(ctx context.Context, cr *dchv1alpha1.DataConnectService, desired []*unstructured.Unstructured) error {
	desiredKeys := make(map[string]struct{}, len(desired))
	for _, resource := range desired {
		desiredKeys[resource.GetKind()+"/"+resource.GetName()] = struct{}{}
	}
	resources := autoscalingResourceNames(cr.Name)
	for _, resource := range resources {
		if _, keep := desiredKeys[resource.GetKind()+"/"+resource.GetName()]; keep {
			continue
		}
		existing := &unstructured.Unstructured{}
		existing.SetGroupVersionKind(resource.GroupVersionKind())
		err := r.Get(ctx, client.ObjectKey{Name: resource.GetName(), Namespace: cr.Namespace}, existing)
		if apierrors.IsNotFound(err) || meta.IsNoMatchError(err) {
			continue
		}
		if err != nil {
			return fmt.Errorf("getting stale autoscaling %s %s: %w", resource.GetKind(), resource.GetName(), err)
		}
		if !isUnstructuredOwnedBy(existing, cr.UID) {
			continue
		}
		if err := r.Delete(ctx, existing); err != nil && !apierrors.IsNotFound(err) {
			return fmt.Errorf("deleting stale autoscaling %s %s: %w", resource.GetKind(), resource.GetName(), err)
		}
	}
	return nil
}

func autoscalingResourceNames(crName string) []*unstructured.Unstructured {
	saName := autoscalingName(crName, autoscalingServiceAccountSuffix)
	secretName := autoscalingName(crName, autoscalingSecretSuffix)
	authName := autoscalingName(crName, autoscalingAuthSuffix)
	roleName := autoscalingName(crName, autoscalingRoleSuffix)
	bindingName := autoscalingName(crName, autoscalingBindingSuffix)
	viewBindingName := autoscalingName(crName, autoscalingViewBindingSuffix)
	return []*unstructured.Unstructured{
		newAutoscalingObject("v1", kindServiceAccount, saName, schema.GroupVersionKind{Version: "v1", Kind: kindServiceAccount}, nil),
		newAutoscalingObject("v1", kindSecret, secretName, schema.GroupVersionKind{Version: "v1", Kind: kindSecret}, nil),
		newAutoscalingObject("keda.sh/v1alpha1", "TriggerAuthentication", authName, triggerAuthGVK, nil),
		newAutoscalingObject(autoscalingRBACGroup+"/v1", kindRole, roleName, schema.GroupVersionKind{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRole}, nil),
		newAutoscalingObject(autoscalingRBACGroup+"/v1", kindRoleBinding, bindingName, schema.GroupVersionKind{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRoleBinding}, nil),
		newAutoscalingObject(autoscalingRBACGroup+"/v1", kindRoleBinding, viewBindingName, schema.GroupVersionKind{Group: autoscalingRBACGroup, Version: "v1", Kind: kindRoleBinding}, nil),
		newAutoscalingObject("keda.sh/v1alpha1", "ScaledObject", autoscalingName(crName, "-rest"+autoscalingScaledObjectSuffix), scaledObjectGVK, nil),
		newAutoscalingObject("keda.sh/v1alpha1", "ScaledObject", autoscalingName(crName, "-flight"+autoscalingScaledObjectSuffix), scaledObjectGVK, nil),
	}
}

func isUnstructuredOwnedBy(obj *unstructured.Unstructured, uid types.UID) bool {
	for _, ref := range obj.GetOwnerReferences() {
		if ref.UID == uid {
			return true
		}
	}
	return false
}
