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
	"bufio"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/pelletier/go-toml/v2"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	apimachtypes "k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/kustomize/api/krusty"
	kustypes "sigs.k8s.io/kustomize/api/types"
	"sigs.k8s.io/kustomize/kyaml/filesys"
	"sigs.k8s.io/kustomize/kyaml/resid"
	sigyaml "sigs.k8s.io/yaml"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// --- Kustomize rendering ---

const (
	// resourceNamePrefix must match namePrefix in config/base/kustomization.yaml.
	resourceNamePrefix = "dch-"

	ParamRestImage           = "RELATED_IMAGE_ODH_DATA_CONNECT_HUB_REST_IMAGE"
	ParamFlightImage         = "RELATED_IMAGE_ODH_DATA_CONNECT_HUB_FLIGHT_IMAGE"
	ParamKubeRbacProxyImage  = "RELATED_IMAGE_ODH_KUBE_RBAC_PROXY_IMAGE"
	ParamFlightResourceName  = "FLIGHT_RESOURCE_NAME"
	ParamFlightInstanceName  = "FLIGHT_INSTANCE_NAME"
	ParamFlightConfigName    = "FLIGHT_CONFIG_NAME"
	ParamFlightSAName        = "FLIGHT_SA_NAME"
	ParamFlightCRBName       = "FLIGHT_CRB_NAME"
	ParamFlightTLSName       = "FLIGHT_TLS_NAME"
	ParamFlightHTTPRouteName = "FLIGHT_HTTPROUTE_NAME"
)

// renderKustomization builds the kustomization at diskPath. The whole of
// rootPath is staged in memory first, not just diskPath, so that a kustomization
// may reference resources outside its own directory (overlays/openshift pulls in
// ../../base). rootPath must contain diskPath. params override values in the
// staged base/params.env before Kustomize resolves replacements.
func renderKustomization(rootPath, diskPath string, patches []kustypes.Patch, images []kustypes.Image, params map[string]string) ([]*unstructured.Unstructured, error) {
	absPath, err := filepath.Abs(diskPath)
	if err != nil {
		return nil, fmt.Errorf("resolving path %s: %w", diskPath, err)
	}
	absRoot, err := filepath.Abs(rootPath)
	if err != nil {
		return nil, fmt.Errorf("resolving root %s: %w", rootPath, err)
	}

	memFS := filesys.MakeFsInMemory()
	if err := copyDirToMemFS(absRoot, memFS); err != nil {
		return nil, fmt.Errorf("copying manifests to memory: %w", err)
	}

	if len(params) > 0 {
		paramsPath := filepath.Join(absRoot, "base", "params.env")
		if err := mergeParamsEnv(memFS, paramsPath, params); err != nil {
			return nil, fmt.Errorf("merging Kustomize params: %w", err)
		}
	}

	if len(patches) > 0 || len(images) > 0 {
		if err := patchKustomization(memFS, absPath, patches, images); err != nil {
			return nil, fmt.Errorf("patching kustomization: %w", err)
		}
	}

	return runKrusty(memFS, absPath)
}

func runKrusty(fs filesys.FileSystem, path string) ([]*unstructured.Unstructured, error) {
	opts := krusty.MakeDefaultOptions()
	k := krusty.MakeKustomizer(opts)

	resMap, err := k.Run(fs, path)
	if err != nil {
		return nil, fmt.Errorf("kustomize run failed for %s: %w", path, err)
	}

	objects := make([]*unstructured.Unstructured, 0, resMap.Size())
	for _, res := range resMap.Resources() {
		jsonBytes, err := res.MarshalJSON()
		if err != nil {
			return nil, fmt.Errorf("marshalling resource %s: %w", res.OrgId(), err)
		}
		obj := &unstructured.Unstructured{}
		if err := obj.UnmarshalJSON(jsonBytes); err != nil {
			return nil, fmt.Errorf("unmarshalling resource: %w", err)
		}
		objects = append(objects, obj)
	}
	return objects, nil
}

func copyDirToMemFS(srcRoot string, memFS filesys.FileSystem) error {
	return filepath.WalkDir(srcRoot, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return memFS.MkdirAll(path)
		}
		if d.Type()&os.ModeSymlink != 0 {
			return nil
		}
		data, err := os.ReadFile(path) //nolint:gosec
		if err != nil {
			return fmt.Errorf("reading %s: %w", path, err)
		}
		return memFS.WriteFile(path, data)
	})
}

// mergeParamsEnv overlays values onto the staged params.env and writes the
// merged entries back. The source file on disk remains untouched.
func mergeParamsEnv(fs filesys.FileSystem, path string, overrides map[string]string) error {
	data, err := fs.ReadFile(path)
	if err != nil {
		return fmt.Errorf("reading %s: %w", path, err)
	}

	params := make(map[string]string)
	scanner := bufio.NewScanner(strings.NewReader(string(data)))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, value, ok := strings.Cut(line, "=")
		if ok {
			params[strings.TrimSpace(key)] = value
		}
	}
	if err := scanner.Err(); err != nil {
		return fmt.Errorf("reading %s: %w", path, err)
	}

	for key, value := range overrides {
		if strings.TrimSpace(value) == "" || strings.ContainsAny(value, "\r\n") {
			return fmt.Errorf("invalid params.env value for key %q", key)
		}
		if _, ok := params[key]; !ok {
			return fmt.Errorf("params.env key %q not found in %s", key, path)
		}
		params[key] = value
	}

	lines := make([]string, 0, len(params))
	for key, value := range params {
		lines = append(lines, key+"="+value)
	}
	if err := fs.WriteFile(path, []byte(strings.Join(lines, "\n")+"\n")); err != nil {
		return fmt.Errorf("writing %s: %w", path, err)
	}
	return nil
}

func patchKustomization(fs filesys.FileSystem, dir string, patches []kustypes.Patch, images []kustypes.Image) error {
	kustPath := filepath.Join(dir, "kustomization.yaml")
	data, err := fs.ReadFile(kustPath)
	if err != nil {
		return fmt.Errorf("reading kustomization: %w", err)
	}

	var kust map[string]any
	if err := sigyaml.Unmarshal(data, &kust); err != nil {
		return fmt.Errorf("parsing kustomization: %w", err)
	}

	if len(patches) > 0 {
		patchBytes, err := json.Marshal(patches)
		if err != nil {
			return err
		}
		var patchSlice []any
		if err := json.Unmarshal(patchBytes, &patchSlice); err != nil {
			return err
		}
		existing, _ := kust["patches"].([]any)
		kust["patches"] = append(existing, patchSlice...)
	}

	if len(images) > 0 {
		imgBytes, err := json.Marshal(images)
		if err != nil {
			return err
		}
		var imgSlice []any
		if err := json.Unmarshal(imgBytes, &imgSlice); err != nil {
			return err
		}
		existing, _ := kust["images"].([]any)
		kust["images"] = append(existing, imgSlice...)
	}

	out, err := sigyaml.Marshal(kust)
	if err != nil {
		return fmt.Errorf("serializing kustomization: %w", err)
	}
	return fs.WriteFile(kustPath, out)
}

// --- CR overrides → kustomize patches ---

func buildServicePatches(name string, overrides *dchv1alpha1.ServiceOverrides) []kustypes.Patch {
	if overrides == nil {
		return nil
	}

	var patches []kustypes.Patch

	var patchParts []string

	if overrides.Replicas != nil {
		patchParts = append(patchParts, fmt.Sprintf("spec:\n  replicas: %d", *overrides.Replicas))
	}

	if overrides.Resources != nil {
		resBytes, err := json.Marshal(overrides.Resources)
		if err == nil {
			resYAML, err := sigyaml.JSONToYAML(resBytes)
			if err == nil {
				patchParts = append(patchParts, fmt.Sprintf("spec:\n  template:\n    spec:\n      containers:\n        - name: %s\n          resources:\n%s",
					name, indent(string(resYAML), 12)))
			}
		}
	}

	if len(overrides.Env) > 0 {
		envBytes, err := json.Marshal(overrides.Env)
		if err == nil {
			envYAML, err := sigyaml.JSONToYAML(envBytes)
			if err == nil {
				patchParts = append(patchParts, fmt.Sprintf("spec:\n  template:\n    spec:\n      containers:\n        - name: %s\n          env:\n%s",
					name, indent(string(envYAML), 12)))
			}
		}
	}

	if len(overrides.EnvFrom) > 0 {
		envFromBytes, err := json.Marshal(overrides.EnvFrom)
		if err == nil {
			envFromYAML, err := sigyaml.JSONToYAML(envFromBytes)
			if err == nil {
				patchParts = append(patchParts, fmt.Sprintf("spec:\n  template:\n    spec:\n      containers:\n        - name: %s\n          envFrom:\n%s",
					name, indent(string(envFromYAML), 12)))
			}
		}
	}

	if len(overrides.VolumeMounts) > 0 {
		vmBytes, err := json.Marshal(overrides.VolumeMounts)
		if err == nil {
			vmYAML, err := sigyaml.JSONToYAML(vmBytes)
			if err == nil {
				patchParts = append(patchParts, fmt.Sprintf("spec:\n  template:\n    spec:\n      containers:\n        - name: %s\n          volumeMounts:\n%s",
					name, indent(string(vmYAML), 12)))
			}
		}
	}

	if len(overrides.Volumes) > 0 {
		volBytes, err := json.Marshal(overrides.Volumes)
		if err == nil {
			volYAML, err := sigyaml.JSONToYAML(volBytes)
			if err == nil {
				patchParts = append(patchParts, fmt.Sprintf("spec:\n  template:\n    spec:\n      volumes:\n%s",
					indent(string(volYAML), 8)))
			}
		}
	}

	for _, part := range patchParts {
		patches = append(patches, kustypes.Patch{
			Target: &kustypes.Selector{
				ResId: resid.ResId{
					Gvk:  resid.Gvk{Group: "apps", Version: "v1", Kind: kindDeployment},
					Name: name,
				},
			},
			Patch: fmt.Sprintf("apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: %s\n%s", name, part),
		})
	}

	return patches
}

func flightInstanceName(crName string) string {
	return crName + "-flight"
}

func httpRouteResourceName(crName string) string {
	return crName + "-route"
}

func parseConfigMapTOML(obj *unstructured.Unstructured) (config map[string]any, data map[string]string, err error) {
	data, found, _ := unstructured.NestedStringMap(obj.Object, "data")
	if !found {
		return nil, nil, nil
	}
	tomlText, ok := data["config.toml"]
	if !ok {
		return nil, nil, nil
	}
	if err := toml.Unmarshal([]byte(tomlText), &config); err != nil {
		return nil, nil, err
	}
	return config, data, nil
}

func setConfigMapTOML(obj *unstructured.Unstructured, config map[string]any, data map[string]string) {
	out, err := toml.Marshal(config)
	if err != nil {
		return
	}
	data["config.toml"] = string(out)
	_ = unstructured.SetNestedStringMap(obj.Object, data, "data")
}

// setConfigMapFlightServiceAddress updates the REST ConfigMap to point to the flight service FQDN.
func setConfigMapFlightServiceAddress(resources []*unstructured.Unstructured, namespace, flightResourceName string) {
	fqdn := fmt.Sprintf("%s.%s.svc", flightResourceName, namespace)
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap {
			continue
		}
		config, data, err := parseConfigMapTOML(obj)
		if err != nil || config == nil {
			continue
		}
		fs, ok := config["flight-service"].(map[string]any)
		if !ok {
			continue
		}
		fs["address"] = fqdn
		setConfigMapTOML(obj, config, data)
	}
}

// setConfigMapFlightConnectorSettings updates the flight ConfigMap with connector overrides.
func setConfigMapFlightConnectorSettings(resources []*unstructured.Unstructured, flightInstanceName string, overrides *dchv1alpha1.ServiceOverrides) error {
	if overrides == nil || len(overrides.Connectors) == 0 {
		return nil
	}

	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap || obj.GetLabels()[labelAppName] != flightInstanceName {
			continue
		}
		config, data, err := parseConfigMapTOML(obj)
		if err != nil {
			return fmt.Errorf("parsing flight-service config.toml: %w", err)
		}
		if config == nil {
			continue
		}
		connectors, ok := config["connectors"].(map[string]any)
		if !ok {
			connectors = make(map[string]any)
			config["connectors"] = connectors
		}

		for _, connector := range overrides.Connectors {
			if connector.Name == "" {
				continue
			}
			section, ok := connectors[connector.Name].(map[string]any)
			if !ok {
				section = make(map[string]any)
				connectors[connector.Name] = section
			}

			section["enabled"] = connector.Enabled != nil && *connector.Enabled
			if connector.ConnectionTimeout != nil {
				duration := connector.ConnectionTimeout.Duration
				if duration <= 0 || duration%time.Second != 0 {
					return fmt.Errorf("connector %s connectionTimeout must be a positive whole number of seconds", connector.Name)
				}
				section["connection_timeout_secs"] = int64(duration / time.Second)
			}
			if connector.RequestTimeout != nil {
				duration := connector.RequestTimeout.Duration
				if duration <= 0 || duration%time.Second != 0 {
					return fmt.Errorf("connector %s requestTimeout must be a positive whole number of seconds", connector.Name)
				}
				section["request_timeout_secs"] = int64(duration / time.Second)
			}
			if connector.ReadTimeout != nil {
				duration := connector.ReadTimeout.Duration
				if duration <= 0 || duration%time.Second != 0 {
					return fmt.Errorf("connector %s readTimeout must be a positive whole number of seconds", connector.Name)
				}
				section["read_timeout_secs"] = int64(duration / time.Second)
			}
		}

		setConfigMapTOML(obj, config, data)
	}
	return nil
}

// setConfigMapGlobalNamespace replaces the default tenant-id in the
// flight-service and rest-service ConfigMaps with the CR namespace.
// setConfigMapGlobalNamespace updates the flight and REST ConfigMaps with the deployment namespace.
func setConfigMapGlobalNamespace(resources []*unstructured.Unstructured, namespace string) {
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap {
			continue
		}
		config, data, err := parseConfigMapTOML(obj)
		if err != nil || config == nil {
			continue
		}
		gct, ok := config["global-connection-types"].(map[string]any)
		if !ok {
			continue
		}
		gct["tenant-id"] = namespace
		setConfigMapTOML(obj, config, data)
	}
}

// reconcileTraceEnv reconciles OTLP trace environment variables in deployments,
// removing stale variables and setting current ones. This ensures that clearing
// trace.insecure or trace.certificate from the CR actually removes the env vars
// from the deployment, preventing stale security-sensitive configuration.
func reconcileTraceEnv(resources []*unstructured.Unstructured, trace *dchv1alpha1.Trace, containerNames ...string) bool {
	wanted := make(map[string]bool, len(containerNames))
	for _, name := range containerNames {
		wanted[name] = true
	}

	// Build the desired env var set
	desiredVars := traceEnv(trace)
	desiredMap := make(map[string]string, len(desiredVars))
	for _, v := range desiredVars {
		desiredMap[v.Name] = v.Value
	}

	// All OTLP env vars that must be reconciled
	otlpVars := map[string]bool{
		envOTLPEndpoint:    true,
		envOTLPInsecure:    true,
		envOTLPCertificate: true,
	}

	updated := false
	for _, obj := range resources {
		if obj.GetKind() != kindDeployment {
			continue
		}
		containers, found, _ := unstructured.NestedSlice(obj.Object, "spec", "template", "spec", "containers")
		if !found {
			continue
		}
		changed := false
		for i, c := range containers {
			container, ok := c.(map[string]any)
			if !ok {
				continue
			}
			if name, _ := container["name"].(string); !wanted[name] {
				continue
			}

			env, _ := container["env"].([]any)
			var newEnv []any

			// Remove all OTLP env vars, preserve others
			for _, e := range env {
				existing, ok := e.(map[string]any)
				if !ok {
					newEnv = append(newEnv, e)
					continue
				}
				if n, _ := existing["name"].(string); !otlpVars[n] {
					newEnv = append(newEnv, e)
				}
			}

			// Add back the desired OTLP env vars
			for name, value := range desiredMap {
				newEnv = append(newEnv, map[string]any{"name": name, "value": value})
			}

			if len(newEnv) != len(env) || len(desiredMap) > 0 {
				container["env"] = newEnv
				containers[i] = container
				changed = true
			}
		}
		if changed {
			_ = unstructured.SetNestedSlice(obj.Object, containers, "spec", "template", "spec", "containers")
			updated = true
		}
	}
	return updated
}

// setDeploymentEnv sets the given environment variables on the named
// containers, overwriting any value the base manifest already declares for the
// same name and appending the rest. It reports whether any container matched.
func setDeploymentEnv(resources []*unstructured.Unstructured, vars []corev1.EnvVar, containerNames ...string) bool {
	wanted := make(map[string]bool, len(containerNames))
	for _, name := range containerNames {
		wanted[name] = true
	}

	updated := false
	for _, obj := range resources {
		if obj.GetKind() != kindDeployment {
			continue
		}
		containers, found, _ := unstructured.NestedSlice(obj.Object, "spec", "template", "spec", "containers")
		if !found {
			continue
		}
		changed := false
		for i, c := range containers {
			container, ok := c.(map[string]any)
			if !ok {
				continue
			}
			if name, _ := container["name"].(string); !wanted[name] {
				continue
			}

			env, _ := container["env"].([]any)
			for _, v := range vars {
				entry := map[string]any{"name": v.Name, "value": v.Value}
				replaced := false
				for j, e := range env {
					existing, ok := e.(map[string]any)
					if !ok {
						continue
					}
					if n, _ := existing["name"].(string); n == v.Name {
						env[j] = entry
						replaced = true
					}
				}
				if !replaced {
					env = append(env, entry)
				}
			}

			container["env"] = env
			containers[i] = container
			changed = true
		}
		if changed {
			_ = unstructured.SetNestedSlice(obj.Object, containers, "spec", "template", "spec", "containers")
			updated = true
		}
	}
	return updated
}

// traceEnv maps spec.trace onto the OTLP exporter environment variables,
// omitting any field the CR leaves unset.
func traceEnv(trace *dchv1alpha1.Trace) []corev1.EnvVar {
	if trace == nil {
		return nil
	}
	var vars []corev1.EnvVar
	if trace.Exporter != "" {
		vars = append(vars, corev1.EnvVar{Name: envOTLPEndpoint, Value: trace.Exporter})
	}
	if trace.Insecure != nil {
		vars = append(vars, corev1.EnvVar{Name: envOTLPInsecure, Value: strconv.FormatBool(*trace.Insecure)})
	}
	if trace.Certificate != "" {
		vars = append(vars, corev1.EnvVar{Name: envOTLPCertificate, Value: trace.Certificate})
	}
	return vars
}

// setConfigMapDiscoveryServiceAccount updates the flight ConfigMap with the REST service account identity.
func setConfigMapDiscoveryServiceAccount(resources []*unstructured.Unstructured, namespace, flightInstanceName string) {
	var restServiceAccount string
	for _, obj := range resources {
		if obj.GetKind() == kindServiceAccount && obj.GetLabels()[labelAppName] == nameRestService {
			restServiceAccount = obj.GetName()
			break
		}
	}
	if restServiceAccount == "" {
		return
	}

	identity := fmt.Sprintf("system:serviceaccount:%s:%s", namespace, restServiceAccount)
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap || obj.GetLabels()[labelAppName] != flightInstanceName {
			continue
		}
		config, data, err := parseConfigMapTOML(obj)
		if err != nil || config == nil {
			continue
		}
		auth, ok := config["auth"].(map[string]any)
		if !ok {
			continue
		}
		auth["discovery_service_account"] = identity
		setConfigMapTOML(obj, config, data)
	}
}

func buildGatewayPatches(gw *dchv1alpha1.Gateway) []kustypes.Patch {
	if gw == nil {
		return nil
	}

	patchYAML := fmt.Sprintf(`apiVersion: gateway.networking.k8s.io/v1
kind: HTTPRoute
metadata:
  name: data-connect-hub
spec:
  parentRefs:
    - name: %s
      namespace: %s`, gw.Name, gw.Namespace)

	return []kustypes.Patch{
		{
			Patch: patchYAML,
		},
	}
}

// --- Apply resources with SSA and owner references ---

// resourcePriority returns a sort key that ensures infrastructure resources
// (ServiceAccount, ConfigMap, …) are applied before workloads (Deployment).
// On OpenShift the SA's dockercfg pull-secret is generated asynchronously;
// creating the Deployment first produces pods without imagePullSecrets.
func resourcePriority(kind string) int {
	switch kind {
	case kindServiceAccount:
		return 0
	case kindConfigMap, kindSecret, kindService, kindNetworkPolicy,
		kindClusterRole, kindClusterRoleBinding, kindRole, kindRoleBinding:
		return 1
	case kindDeployment, kindStatefulSet, kindDaemonSet, kindJob:
		return 2
	default:
		return 3
	}
}

func (r *DataConnectServiceReconciler) applyResources(
	ctx context.Context,
	cr *dchv1alpha1.DataConnectService,
	namespace string,
	resources []*unstructured.Unstructured,
) error {
	log := logf.FromContext(ctx)

	slices.SortStableFunc(resources, func(a, b *unstructured.Unstructured) int {
		return resourcePriority(a.GetKind()) - resourcePriority(b.GetKind())
	})

	for _, obj := range resources {
		obj.SetNamespace(namespace)

		if obj.GetKind() == kindClusterRoleBinding {
			patchClusterRoleBindingSubjects(obj, namespace)
		}

		labels := obj.GetLabels()
		if labels == nil {
			labels = map[string]string{}
		}
		labels[labelManagedBy] = managedByDCHService
		obj.SetLabels(labels)

		if err := controllerutil.SetControllerReference(cr, obj, r.Scheme); err != nil {
			return fmt.Errorf("setting owner ref on %s %s: %w", obj.GetKind(), obj.GetName(), err)
		}

		desiredHash := specHash(obj)
		ann := obj.GetAnnotations()
		if ann == nil {
			ann = map[string]string{}
		}
		ann[annotationSpecHash] = desiredHash
		obj.SetAnnotations(ann)

		existing := &unstructured.Unstructured{}
		existing.SetGroupVersionKind(obj.GroupVersionKind())
		err := r.Get(ctx, client.ObjectKeyFromObject(obj), existing)

		if apierrors.IsNotFound(err) {
			if err := r.Create(ctx, obj); err != nil {
				if apierrors.IsAlreadyExists(err) {
					continue
				}
				return fmt.Errorf("creating %s %s: %w", obj.GetKind(), obj.GetName(), err)
			}
			log.V(1).Info("created resource", "kind", obj.GetKind(), "name", obj.GetName())
			continue
		}
		if err != nil {
			return fmt.Errorf("getting %s %s: %w", obj.GetKind(), obj.GetName(), err)
		}

		existingHash := ""
		if existingAnn := existing.GetAnnotations(); existingAnn != nil {
			existingHash = existingAnn[annotationSpecHash]
		}
		if existingHash == desiredHash {
			if !hasControllerOwner(existing, cr.GetUID()) {
				if err := controllerutil.SetControllerReference(cr, existing, r.Scheme); err == nil {
					if updateErr := r.Update(ctx, existing); updateErr != nil {
						return fmt.Errorf("repairing owner ref on %s %s: %w", obj.GetKind(), obj.GetName(), updateErr)
					}
				}
			}
			continue
		}

		// Spec changed or first reconcile — apply via SSA.
		obj.SetResourceVersion("")
		obj.SetManagedFields(nil)
		if err := r.Patch(ctx, obj, client.Apply, client.FieldOwner("dc-controller"), client.ForceOwnership); err != nil { //nolint:staticcheck // client.Apply is the standard SSA approach for unstructured objects
			return fmt.Errorf("updating %s %s: %w", obj.GetKind(), obj.GetName(), err)
		}
		log.V(1).Info("updated resource", "kind", obj.GetKind(), "name", obj.GetName())
	}
	return nil
}

func hasControllerOwner(obj *unstructured.Unstructured, uid apimachtypes.UID) bool {
	for _, ref := range obj.GetOwnerReferences() {
		if ref.UID == uid && ref.Controller != nil && *ref.Controller {
			return true
		}
	}
	return false
}

func specHash(obj *unstructured.Unstructured) string {
	content := obj.DeepCopy().UnstructuredContent()
	delete(content, "status")
	if md, ok := content["metadata"].(map[string]any); ok {
		delete(md, "resourceVersion")
		delete(md, "uid")
		delete(md, "creationTimestamp")
		delete(md, "generation")
		delete(md, "managedFields")
		delete(md, "ownerReferences")
		delete(md, "annotations")
	}
	b, _ := json.Marshal(content)
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:])[:16]
}

// --- Database secret validation ---

func (r *DataConnectServiceReconciler) validateDatabaseSecret(ctx context.Context, namespace string) error {
	secret := &corev1.Secret{}
	key := client.ObjectKey{Name: nameDatabaseConfig, Namespace: namespace}
	if err := r.Get(ctx, key, secret); err != nil {
		if apierrors.IsNotFound(err) {
			return fmt.Errorf("secret %q not found in namespace %q — it must contain a secret-config.toml key", nameDatabaseConfig, namespace)
		}
		return fmt.Errorf("reading secret %s: %w", nameDatabaseConfig, err)
	}

	for _, k := range []string{"secret-config.toml"} {
		value, ok := secret.Data[k]
		if !ok || strings.TrimSpace(string(value)) == "" {
			return fmt.Errorf("secret %q is missing or has empty required key %q", nameDatabaseConfig, k)
		}
	}
	return nil
}

func patchClusterRoleBindingSubjects(obj *unstructured.Unstructured, namespace string) {
	subjects, found, _ := unstructured.NestedSlice(obj.Object, "subjects")
	if !found {
		return
	}
	for i, s := range subjects {
		sub, ok := s.(map[string]any)
		if !ok {
			continue
		}
		if kind, _ := sub["kind"].(string); kind == kindServiceAccount {
			sub["namespace"] = namespace
			subjects[i] = sub
		}
	}
	_ = unstructured.SetNestedSlice(obj.Object, subjects, "subjects")
}

// setConfigMapAudiences updates the flight ConfigMap with token review audiences.
func setConfigMapAudiences(resources []*unstructured.Unstructured, audiences []string) bool {
	updated := false
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap {
			continue
		}
		config, data, err := parseConfigMapTOML(obj)
		if err != nil || config == nil {
			continue
		}
		auth, ok := config["auth"].(map[string]any)
		if !ok {
			continue
		}
		auth["token_review_audiences"] = audiences
		setConfigMapTOML(obj, config, data)
		updated = true
	}
	return updated
}

func setKubeRbacProxyAudiences(resources []*unstructured.Unstructured, audiences []string) {
	arg := fmt.Sprintf("--auth-token-audiences=%s", strings.Join(audiences, ","))
	for _, obj := range resources {
		if obj.GetKind() != kindDeployment {
			continue
		}
		containers, found, _ := unstructured.NestedSlice(obj.Object, "spec", "template", "spec", "containers")
		if !found {
			continue
		}
		for i, c := range containers {
			container, ok := c.(map[string]any)
			if !ok {
				continue
			}
			name, _ := container["name"].(string)
			if name != nameKubeRbacProxy {
				continue
			}
			var args []any
			if existing, ok := container["args"].([]any); ok {
				args = existing
			}
			args = append(args, arg)
			container["args"] = args
			containers[i] = container
		}
		_ = unstructured.SetNestedSlice(obj.Object, containers, "spec", "template", "spec", "containers")
	}
}

func annotateDeploymentWithConfigHash(resources []*unstructured.Unstructured, containerName, configMapSuffix string) {
	var configHash string
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap || !strings.HasSuffix(obj.GetName(), configMapSuffix) {
			continue
		}
		data, found, _ := unstructured.NestedStringMap(obj.Object, "data")
		if !found {
			continue
		}
		b, _ := json.Marshal(data)
		h := sha256.Sum256(b)
		configHash = hex.EncodeToString(h[:])[:16]
		break
	}
	if configHash == "" {
		return
	}

	for _, obj := range resources {
		if obj.GetKind() != kindDeployment {
			continue
		}
		containers, found, _ := unstructured.NestedSlice(obj.Object, "spec", "template", "spec", "containers")
		if !found {
			continue
		}
		hasContainer := false
		for _, c := range containers {
			if container, ok := c.(map[string]any); ok {
				if name, _ := container["name"].(string); name == containerName {
					hasContainer = true
					break
				}
			}
		}
		if !hasContainer {
			continue
		}
		ann, _, _ := unstructured.NestedStringMap(obj.Object, "spec", "template", "metadata", "annotations")
		if ann == nil {
			ann = map[string]string{}
		}
		ann[annotationConfigHash] = configHash
		_ = unstructured.SetNestedStringMap(obj.Object, ann, "spec", "template", "metadata", "annotations")
	}
}

func annotateFlightDeploymentsWithConfigHash(resources []*unstructured.Unstructured, flightInstanceName, containerName string) {
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap || obj.GetLabels()[labelAppName] != flightInstanceName {
			continue
		}
		annotateDeploymentWithConfigHash(resources, containerName, obj.GetName())
	}
}

func indent(s string, spaces int) string {
	pad := strings.Repeat(" ", spaces)
	lines := strings.Split(strings.TrimRight(s, "\n"), "\n")
	for i, line := range lines {
		if line != "" {
			lines[i] = pad + line
		}
	}
	return strings.Join(lines, "\n")
}
