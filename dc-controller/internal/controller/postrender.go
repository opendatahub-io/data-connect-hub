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
	"fmt"
	"strings"
	"time"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	logf "sigs.k8s.io/controller-runtime/pkg/log"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// overlayParams packages the values that the config overlays need to inject
// into rendered ConfigMaps: those derived from the CR and platform
// configuration, plus identities resolved from the rendered resources.
type overlayParams struct {
	Namespace string
	// FlightInstanceName is the CR-specific instance name of the flight
	// service ("<cr-name>-flight"), distinct from the static service name
	// (nameFlightService). After renderFlightService, the flight ConfigMap
	// label and Service name suffix match this value.
	FlightInstanceName string
	Audiences          []string
	Connectors         []dchv1alpha1.ConnectorConfig

	// FlightFQDN is the cluster-local address of the flight Service,
	// resolved from the rendered resources.
	FlightFQDN string

	// RestServiceAccount is the rest-service ServiceAccount identity in
	// system:serviceaccount:<ns>:<name> form, resolved from the rendered
	// resources.
	RestServiceAccount string
}

// postRender applies all post-kustomize mutations to the rendered resources:
// config overlay injection, trace env vars, kube-rbac-proxy audiences,
// and content-hash annotations for rollout detection.
func (r *DataConnectServiceReconciler) postRender(
	ctx context.Context,
	resources []*unstructured.Unstructured,
	cr *dchv1alpha1.DataConnectService,
	platCfg *platformConfig,
) error {
	log := logf.FromContext(ctx)

	resources = renderFlightService(resources, cr.Name)

	params := r.buildOverlayParams(cr, platCfg, resources)

	if err := injectConfigOverlays(resources, params); err != nil {
		return fmt.Errorf("injecting config overlay values: %w", err)
	}

	if !reconcileTraceEnv(resources, cr.Spec.Trace, nameRestServiceContainer, nameFlightServiceContainer) {
		log.V(1).Info("trace reconciliation: no service container found in rendered manifests")
	}

	if len(params.Audiences) > 0 {
		setKubeRbacProxyAudiences(resources, params.Audiences)
	}

	if err := r.annotateDeploymentsWithContentHash(ctx, resources, cr.Namespace); err != nil {
		return fmt.Errorf("annotating deployments with content hash: %w", err)
	}
	return nil
}

// buildOverlayParams assembles the overlay parameters from the CR, platform
// configuration, and identities resolved from the rendered resources.
func (r *DataConnectServiceReconciler) buildOverlayParams(
	cr *dchv1alpha1.DataConnectService,
	platCfg *platformConfig,
	resources []*unstructured.Unstructured,
) overlayParams {
	params := overlayParams{
		Namespace:          cr.Namespace,
		FlightInstanceName: flightServiceResourceName(cr.Name),
		Audiences:          r.resolveTokenReviewAudiences(cr, platCfg),
	}
	if cr.Spec.FlightService != nil {
		params.Connectors = cr.Spec.FlightService.Connectors
	}

	// Resolve the Flight service FQDN and rest-service ServiceAccount
	// identity from the rendered resources.
	for _, obj := range resources {
		switch {
		case obj.GetKind() == kindService && strings.HasSuffix(obj.GetName(), params.FlightInstanceName):
			params.FlightFQDN = fmt.Sprintf("%s.%s.svc", obj.GetName(), params.Namespace)
		case obj.GetKind() == kindServiceAccount && strings.HasSuffix(obj.GetName(), nameRestService+"-sa"):
			params.RestServiceAccount = fmt.Sprintf("system:serviceaccount:%s:%s", params.Namespace, obj.GetName())
		}
	}
	return params
}

// injectConfigOverlays applies type-safe overlays to the rest-service and
// flight-service ConfigMaps in the rendered resources, matched explicitly by
// label.
func injectConfigOverlays(resources []*unstructured.Unstructured, params overlayParams) error {
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap {
			continue
		}

		config, data, err := loadConfigMapTOML(obj)
		if err != nil {
			return fmt.Errorf("ConfigMap %q: %w", obj.GetName(), err)
		}
		if config == nil {
			continue
		}

		var overlay map[string]any
		switch obj.GetLabels()[labelAppName] {
		case nameRestService:
			overlay, err = buildRestOverlay(params)
			if err != nil {
				return fmt.Errorf("rest-service ConfigMap %q: %w", obj.GetName(), err)
			}
		case params.FlightInstanceName:
			overlay, err = buildFlightOverlay(params)
			if err != nil {
				return fmt.Errorf("flight-service ConfigMap %q: %w", obj.GetName(), err)
			}
		default:
			continue
		}

		mergeConfigOverlay(config, overlay)
		if err := setConfigMapTOML(obj, config, data); err != nil {
			return fmt.Errorf("ConfigMap %q: %w", obj.GetName(), err)
		}
	}
	return nil
}

// buildRestOverlay constructs the overlay values for the rest-service
// ConfigMap. The config.toml provides defaults; overlayParams provide
// overrides — existing values are replaced and missing sections are added.
//
//nolint:unparam // the error result is kept for symmetry with buildFlightOverlay
func buildRestOverlay(params overlayParams) (map[string]any, error) {
	overlay := map[string]any{
		"global-connection-types": map[string]any{
			"tenant-id": params.Namespace,
		},
	}
	if params.FlightFQDN != "" {
		overlay["flight-service"] = map[string]any{"address": params.FlightFQDN}
	}
	return overlay, nil
}

// buildFlightOverlay constructs the overlay values for the flight-service
// ConfigMap. The config.toml provides defaults; overlayParams provide
// overrides — existing values are replaced and missing sections are added.
// Keys mirror the Rust ServerConfig fields.
func buildFlightOverlay(params overlayParams) (map[string]any, error) {
	overlay := map[string]any{
		"global-connection-types": map[string]any{
			"tenant-id": params.Namespace,
		},
	}

	auth := map[string]any{}
	if params.RestServiceAccount != "" {
		auth["discovery_service_account"] = params.RestServiceAccount
	}
	if len(params.Audiences) > 0 {
		auth["token_review_audiences"] = params.Audiences
	}
	if len(auth) > 0 {
		overlay["auth"] = auth
	}

	if len(params.Connectors) > 0 {
		connectors := make(map[string]any, len(params.Connectors))
		for _, c := range params.Connectors {
			if c.Name == "" {
				continue
			}
			section := map[string]any{"enabled": c.Enabled != nil && *c.Enabled}
			for _, t := range []struct {
				duration *metav1.Duration
				key      string
				field    string
			}{
				{c.ConnectionTimeout, "connection_timeout_secs", "connectionTimeout"},
				{c.RequestTimeout, "request_timeout_secs", "requestTimeout"},
				{c.ReadTimeout, "read_timeout_secs", "readTimeout"},
			} {
				secs, err := wholeSeconds(t.duration, t.field, c.Name)
				if err != nil {
					return nil, err
				}
				if secs != nil {
					section[t.key] = *secs
				}
			}
			connectors[c.Name] = section
		}
		overlay["connectors"] = connectors
	}

	return overlay, nil
}

func wholeSeconds(d *metav1.Duration, field, connector string) (*int64, error) {
	if d == nil {
		return nil, nil
	}
	if d.Duration <= 0 || d.Duration%time.Second != 0 {
		return nil, fmt.Errorf("connector %s %s must be a positive whole number of seconds", connector, field)
	}
	secs := int64(d.Duration / time.Second)
	return &secs, nil
}

// mergeConfigOverlay deep-merges overlay into dst, preserving existing
// values in dst that the overlay does not override.
func mergeConfigOverlay(dst, overlay map[string]any) {
	for k, v := range overlay {
		if table, ok := v.(map[string]any); ok {
			if existing, ok := dst[k].(map[string]any); ok {
				mergeConfigOverlay(existing, table)
				continue
			}
		}
		dst[k] = v
	}
}

func flightServiceResourceName(crName string) string {
	return crName + "-flight"
}

func httpRouteResourceName(crName string) string {
	return crName + "-route"
}

// renderFlightService renames the flight-service resources rendered by
// Kustomize to the CR-specific instance name, e.g. "flight-service" becomes
// "<cr-name>-flight". It is the first post-render step applied by postRender.
func renderFlightService(resources []*unstructured.Unstructured, crName string) []*unstructured.Unstructured {
	serviceName := flightServiceResourceName(crName)
	for _, obj := range resources {
		if isFlightServiceResource(obj) {
			renameFlightServiceResource(obj, serviceName)
			continue
		}
		if obj.GetKind() == kindHTTPRoute {
			obj.SetName(httpRouteResourceName(crName))
			obj.Object = replaceStringValue(obj.UnstructuredContent(), nameFlightService, serviceName).(map[string]any)
		}
	}
	return resources
}

func isFlightServiceResource(obj *unstructured.Unstructured) bool {
	name := obj.GetName()
	switch obj.GetKind() {
	case kindConfigMap:
		// The REST service mounts the flight-service-ca ConfigMap, so its
		// name contains "flight-service" even though it is not a Flight
		// service resource. Use the app label to distinguish the Flight
		// ConfigMap from the REST-owned CA ConfigMap.
		return obj.GetLabels()[labelAppName] == nameFlightService
	case kindDeployment, kindService, kindServiceAccount, kindNetworkPolicy:
		return strings.Contains(name, nameFlightService)
	case kindClusterRoleBinding:
		return strings.HasSuffix(name, "flight-auth-delegator")
	default:
		return false
	}
}

func renameFlightServiceResource(obj *unstructured.Unstructured, serviceName string) {
	content := replaceStringValue(obj.UnstructuredContent(), nameFlightService, serviceName).(map[string]any)
	obj.Object = content
	if obj.GetKind() == kindClusterRoleBinding {
		obj.SetName(strings.Replace(obj.GetName(), "flight-auth-delegator", serviceName+"-auth-delegator", 1))
	}
}

func replaceStringValue(value any, old, new string) any {
	switch value := value.(type) {
	case string:
		return strings.ReplaceAll(value, old, new)
	case map[string]any:
		for key, child := range value {
			value[key] = replaceStringValue(child, old, new)
		}
	case []any:
		for i, child := range value {
			value[i] = replaceStringValue(child, old, new)
		}
	}
	return value
}
