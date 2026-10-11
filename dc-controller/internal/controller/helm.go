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
	"bytes"
	"fmt"
	"io"
	"regexp"
	"strconv"
	"strings"
	"time"

	"helm.sh/helm/v3/pkg/chart/loader"
	"helm.sh/helm/v3/pkg/chartutil"
	"helm.sh/helm/v3/pkg/engine"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/util/yaml"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// Helm values keys and defaults shared by the value builders below.
const (
	keyEnabled              = "enabled"
	keyName                 = "name"
	keyResources            = "resources"
	keyLimits               = "limits"
	keyVolumes              = "volumes"
	keyValue                = "value"
	keySecretName           = "secretName"
	keyTokenReviewAudiences = "tokenReviewAudiences"
	keyCPU                  = "cpu"
	keyMemory               = "memory"
	keyRequests             = "requests"

	rustLogEnvName      = "RUST_LOG"
	defaultRustLogLevel = "info"
	defaultMemory256Mi  = "256Mi"
)

type HelmRenderer struct {
	chartPath string
}

func NewHelmRenderer(chartPath string) *HelmRenderer {
	return &HelmRenderer{chartPath: chartPath}
}

type RenderOptions struct {
	MonitoringAvailable bool
}

// flightServiceResourceName derives the flight instance name from the CR
// name, preserving the resource naming used before the chart migration
// (dch-<crName>-flight).
func flightServiceResourceName(crName string) string {
	return crName + "-flight"
}

// httpRouteResourceName derives the HTTPRoute name from the CR name,
// following the same dch-<crName>-... convention as the other operand
// resources. The chart renders the route under this name and the CR status
// reports it, keeping a single source of truth.
//
// This name is stable across the planned multi-instance evolution: flight
// instances are not interchangeable (each serves a different connector set),
// so they will each get their own hostname-based route; this route keeps its
// name and sheds the gRPC rules to the per-instance routes.
func httpRouteResourceName(crName string) string {
	return "dch-" + crName + "-route"
}

func (h *HelmRenderer) RenderChart(
	values map[string]any,
	namespace string,
) ([]*unstructured.Unstructured, error) {
	c, err := loader.Load(h.chartPath)
	if err != nil {
		return nil, fmt.Errorf("loading chart from %s: %w", h.chartPath, err)
	}

	releaseOpts := chartutil.ReleaseOptions{
		Name:      "dch",
		Namespace: namespace,
		IsInstall: true,
	}
	valuesToRender, err := chartutil.ToRenderValues(c, values, releaseOpts, nil)
	if err != nil {
		return nil, fmt.Errorf("preparing render values: %w", err)
	}

	rendered, err := engine.Render(c, valuesToRender)
	if err != nil {
		return nil, fmt.Errorf("rendering templates: %w", err)
	}

	return parseRenderedTemplates(rendered)
}

func parseRenderedTemplates(rendered map[string]string) ([]*unstructured.Unstructured, error) {
	var objects []*unstructured.Unstructured

	for name, content := range rendered {
		if strings.HasSuffix(name, "NOTES.txt") || strings.HasSuffix(name, "_helpers.tpl") {
			continue
		}
		trimmed := strings.TrimSpace(content)
		if trimmed == "" {
			continue
		}

		decoder := yaml.NewYAMLToJSONDecoder(bytes.NewBufferString(trimmed))
		for {
			obj := &unstructured.Unstructured{}
			if err := decoder.Decode(&obj.Object); err != nil {
				if err == io.EOF {
					break
				}
				return nil, fmt.Errorf("decoding %s: %w", name, err)
			}
			if obj.Object == nil {
				continue
			}
			objects = append(objects, obj)
		}
	}
	return objects, nil
}

// buildHelmValues maps the CR spec, platform config, and reconciler state
// into the values map consumed by the data-connect-hub Helm chart.
func (r *DataConnectServiceReconciler) buildHelmValues(
	cr *dchv1alpha1.DataConnectService,
	platCfg *platformConfig,
	opts RenderOptions,
) (map[string]any, error) {
	gw := r.resolveGateway(cr, platCfg)
	audiences := r.resolveTokenReviewAudiences(cr, platCfg)

	values := map[string]any{
		"namePrefix":    "dch-",
		"httpRouteName": httpRouteResourceName(cr.Name),
		"images": map[string]any{
			"restService":   r.RestImage,
			"flightService": r.FlightImage,
			"kubeRbacProxy": r.KubeRbacProxyImage,
		},
		"monitoring": map[string]any{
			keyEnabled:       opts.MonitoringAvailable,
			"scrapeInterval": "30s",
		},
		"gateway": map[string]any{
			keyEnabled: true,
			"parentRef": map[string]any{
				keyName:     gw.Name,
				"namespace": gw.Namespace,
			},
		},
		"trace": buildTraceValues(cr.Spec.Trace),
	}

	// Empty when the controller namespace cannot be determined (e.g. running
	// outside a cluster); the chart's controllerNamespace default then applies.
	if r.ControllerNamespace != "" {
		values["controllerNamespace"] = r.ControllerNamespace
	}

	restValues, err := buildRestServiceValues(cr, audiences)
	if err != nil {
		return nil, fmt.Errorf("rest service values: %w", err)
	}
	values["restService"] = restValues
	values["kubeRbacProxy"] = buildKubeRbacProxyValues(audiences)

	flightValues, err := buildFlightServiceValues(cr, audiences)
	if err != nil {
		return nil, fmt.Errorf("flight service values: %w", err)
	}
	values["flightService"] = flightValues

	return values, nil
}

func buildRestServiceValues(cr *dchv1alpha1.DataConnectService, audiences []string) (map[string]any, error) {
	restValues := map[string]any{
		"replicas": int64(1),
		keyResources: map[string]any{
			keyRequests: map[string]any{keyCPU: "100m", keyMemory: defaultMemory256Mi},
			keyLimits:   map[string]any{keyCPU: "1", keyMemory: "512Mi"},
		},
		"env":          []any{},
		"envFrom":      []any{},
		keyVolumes:     []any{},
		"volumeMounts": []any{},
	}

	if cr.Spec.RestService != nil {
		if err := applyServiceOverrides(restValues, cr.Spec.RestService); err != nil {
			return nil, err
		}
	}

	// Ensure a RUST_LOG default unless the CR provides one. The chart no
	// longer hardcodes it: a hardcoded entry plus a CR-provided entry with the
	// same name produces duplicate env keys, which server-side apply rejects.
	restValues["env"] = withDefaultRustLogEnv(restValues["env"].([]any))

	configValues := map[string]any{
		keyTokenReviewAudiences: stringsToInterfaceSlice(audiences),
	}
	restValues["config"] = configValues

	return restValues, nil
}

func buildKubeRbacProxyValues(audiences []string) map[string]any {
	return map[string]any{
		keyResources: map[string]any{
			keyRequests: map[string]any{keyCPU: "50m", keyMemory: "64Mi"},
			keyLimits:   map[string]any{keyCPU: "500m", keyMemory: defaultMemory256Mi},
		},
		keyTokenReviewAudiences: stringsToInterfaceSlice(audiences),
	}
}

// buildFlightServiceValues maps the CR's single flightService override block
// into the chart's flightService values. When the CR omits the flightService
// section, defaults are used so that Flight is always deployed.
func buildFlightServiceValues(cr *dchv1alpha1.DataConnectService, audiences []string) (map[string]any, error) {
	flight := cr.Spec.FlightService
	if flight == nil {
		flight = &dchv1alpha1.FlightServiceConfig{}
	}

	f := map[string]any{
		keyName:    flightServiceResourceName(cr.Name),
		"replicas": int64(1),
		keyResources: map[string]any{
			keyRequests: map[string]any{keyCPU: "100m", keyMemory: defaultMemory256Mi},
			keyLimits:   map[string]any{keyCPU: "1", keyMemory: "512Mi"},
		},
		"env":          []any{},
		"envFrom":      []any{},
		keyVolumes:     []any{},
		"volumeMounts": []any{},
	}

	overrides := &flight.ServiceOverrides
	if err := applyServiceOverrides(f, overrides); err != nil {
		return nil, err
	}
	f["env"] = withDefaultRustLogEnv(f["env"].([]any))

	configValues := map[string]any{
		keyTokenReviewAudiences: stringsToInterfaceSlice(audiences),
	}
	f["config"] = configValues

	connectors := buildConnectorsMap(flight.Connectors)
	if len(connectors) > 0 {
		f["connectors"] = connectors
	}

	return f, nil
}

var validConnectorName = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]{0,62}$`)

func buildConnectorsMap(configs []dchv1alpha1.ConnectorConfig) map[string]any {
	if len(configs) == 0 {
		return nil
	}
	connectors := make(map[string]any, len(configs))
	for _, cc := range configs {
		if cc.Name == "" || cc.Name == "default" || !validConnectorName.MatchString(cc.Name) {
			continue
		}
		section := map[string]any{}

		if cc.Enabled != nil {
			section["enabled"] = *cc.Enabled
		}
		if cc.ConnectionTimeout != nil {
			d := cc.ConnectionTimeout.Duration
			if d > 0 && d%time.Second == 0 {
				section["connection_timeout_secs"] = int64(d / time.Second)
			}
		}
		if cc.RequestTimeout != nil {
			d := cc.RequestTimeout.Duration
			if d > 0 && d%time.Second == 0 {
				section["request_timeout_secs"] = int64(d / time.Second)
			}
		}
		if cc.ReadTimeout != nil {
			d := cc.ReadTimeout.Duration
			if d > 0 && d%time.Second == 0 {
				section["read_timeout_secs"] = int64(d / time.Second)
			}
		}
		connectors[cc.Name] = section
	}
	return connectors
}

func buildTraceValues(trace *dchv1alpha1.Trace) map[string]any {
	v := map[string]any{
		"endpoint":    "",
		"insecure":    "",
		"certificate": "",
	}
	if trace == nil {
		return v
	}
	if trace.Exporter != "" {
		v["endpoint"] = trace.Exporter
	}
	if trace.Insecure != nil {
		v["insecure"] = strconv.FormatBool(*trace.Insecure)
	}
	if trace.Certificate != "" {
		v["certificate"] = trace.Certificate
	}
	return v
}

func stringsToInterfaceSlice(ss []string) []any {
	if len(ss) == 0 {
		return []any{}
	}
	result := make([]any, len(ss))
	for i, s := range ss {
		result[i] = s
	}
	return result
}

// toUnstructuredSlice converts a slice of Kubernetes API objects to
// unstructured maps, preserving all fields. Follows the org convention
// established in mlflow-operator and opendatahub-operator.
func toUnstructuredSlice[T any](items []T) ([]any, error) {
	out := make([]any, 0, len(items))
	for i := range items {
		m, err := runtime.DefaultUnstructuredConverter.ToUnstructured(&items[i])
		if err != nil {
			return nil, fmt.Errorf("converting item[%d]: %w", i, err)
		}
		out = append(out, m)
	}
	return out, nil
}

func validateVolumes(vols []corev1.Volume) error {
	// Allow only known-safe volume types; reject everything else
	// (hostPath, NFS, flexVolume, gitRepo, image, ...).
	for _, v := range vols {
		src := v.VolumeSource
		allowed := src.ConfigMap != nil || src.Secret != nil || src.EmptyDir != nil ||
			src.PersistentVolumeClaim != nil || src.Projected != nil || src.Ephemeral != nil ||
			src.DownwardAPI != nil || src.CSI != nil
		if !allowed {
			return fmt.Errorf("volume %q: volume source type is not allowed", v.Name)
		}
	}
	return nil
}

func applyServiceOverrides(values map[string]any, overrides *dchv1alpha1.ServiceOverrides) error {
	if overrides.Replicas != nil {
		values["replicas"] = int64(*overrides.Replicas)
	}
	if overrides.Resources != nil {
		m, err := runtime.DefaultUnstructuredConverter.ToUnstructured(overrides.Resources)
		if err != nil {
			return fmt.Errorf("converting resources: %w", err)
		}
		values[keyResources] = m
	}
	if len(overrides.Env) > 0 {
		envSlice, err := toUnstructuredSlice(overrides.Env)
		if err != nil {
			return fmt.Errorf("converting env: %w", err)
		}
		values["env"] = envSlice
	}
	if len(overrides.EnvFrom) > 0 {
		envFromSlice, err := toUnstructuredSlice(overrides.EnvFrom)
		if err != nil {
			return fmt.Errorf("converting envFrom: %w", err)
		}
		values["envFrom"] = envFromSlice
	}
	if len(overrides.Volumes) > 0 {
		if err := validateVolumes(overrides.Volumes); err != nil {
			return err
		}
		volSlice, err := toUnstructuredSlice(overrides.Volumes)
		if err != nil {
			return fmt.Errorf("converting volumes: %w", err)
		}
		values[keyVolumes] = volSlice
	}
	if len(overrides.VolumeMounts) > 0 {
		vmSlice, err := toUnstructuredSlice(overrides.VolumeMounts)
		if err != nil {
			return fmt.Errorf("converting volumeMounts: %w", err)
		}
		values["volumeMounts"] = vmSlice
	}
	return nil
}

// withDefaultRustLogEnv returns env with a default RUST_LOG entry prepended
// when no RUST_LOG entry is present. Rendered env lists must not contain
// duplicate names, otherwise server-side apply rejects the resulting patch.
func withDefaultRustLogEnv(env []any) []any {
	for _, e := range env {
		if entry, ok := e.(map[string]any); ok && entry[keyName] == rustLogEnvName {
			return env
		}
	}
	withDefault := map[string]any{keyName: rustLogEnvName, keyValue: defaultRustLogLevel}
	return append([]any{withDefault}, env...)
}
