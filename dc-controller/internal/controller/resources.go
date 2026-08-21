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
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"

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

func renderKustomization(diskPath string, patches []kustypes.Patch, images []kustypes.Image) ([]*unstructured.Unstructured, error) {
	absPath, err := filepath.Abs(diskPath)
	if err != nil {
		return nil, fmt.Errorf("resolving path %s: %w", diskPath, err)
	}

	memFS := filesys.MakeFsInMemory()
	if err := copyDirToMemFS(absPath, memFS); err != nil {
		return nil, fmt.Errorf("copying manifests to memory: %w", err)
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

	if overrides.ImagePullSecrets != nil {
		ipsBytes, err := json.Marshal(overrides.ImagePullSecrets)
		if err == nil {
			ipsYAML, err := sigyaml.JSONToYAML(ipsBytes)
			if err == nil {
				patchParts = append(patchParts, fmt.Sprintf("spec:\n  template:\n    spec:\n      imagePullSecrets:\n%s",
					indent(string(ipsYAML), 8)))
			}
		}
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

func resolveServiceImage(name string, overrides *dchv1alpha1.ServiceOverrides, restImage, flightImage string) string {
	if overrides != nil && overrides.Image != nil {
		return *overrides.Image
	}
	if name == nameRestService {
		return restImage
	}
	return flightImage
}

func setDeploymentImage(resources []*unstructured.Unstructured, containerName, image string) {
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
			if name, ok := container["name"].(string); ok && name == containerName {
				container["image"] = image
				containers[i] = container
			}
		}
		_ = unstructured.SetNestedSlice(obj.Object, containers, "spec", "template", "spec", "containers")
	}
}

func setConfigMapGlobalNamespace(resources []*unstructured.Unstructured, namespace string) {
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap {
			continue
		}
		data, found, _ := unstructured.NestedStringMap(obj.Object, "data")
		if !found {
			continue
		}
		toml, ok := data["config.toml"]
		if !ok || !strings.Contains(toml, "tenant-id") {
			continue
		}
		data["config.toml"] = strings.ReplaceAll(toml,
			`tenant-id = "opendatahub"`,
			fmt.Sprintf(`tenant-id = "%s"`, namespace))
		_ = unstructured.SetNestedStringMap(obj.Object, data, "data")
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
	case "ServiceAccount":
		return 0
	case "ConfigMap", "Secret", "Service", "NetworkPolicy",
		"ClusterRole", "ClusterRoleBinding", "Role", "RoleBinding":
		return 1
	case kindDeployment, "StatefulSet", "DaemonSet", "Job":
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

		if obj.GetKind() == "ClusterRoleBinding" {
			patchClusterRoleBindingSubjects(obj, namespace)
		}

		labels := obj.GetLabels()
		if labels == nil {
			labels = map[string]string{}
		}
		labels["dataconnecthub.opendatahub.io/managed-by"] = "dataconnectservice"
		obj.SetLabels(labels)

		if err := controllerutil.SetControllerReference(cr, obj, r.Scheme); err != nil {
			return fmt.Errorf("setting owner ref on %s %s: %w", obj.GetKind(), obj.GetName(), err)
		}

		desiredHash := specHash(obj)
		ann := obj.GetAnnotations()
		if ann == nil {
			ann = map[string]string{}
		}
		ann["dataconnecthub/spec-hash"] = desiredHash
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
			existingHash = existingAnn["dataconnecthub/spec-hash"]
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
			return fmt.Errorf("secret %q not found in namespace %q — create it with keys DATABASE_URL and secret-config.toml", nameDatabaseConfig, namespace)
		}
		return fmt.Errorf("reading secret %s: %w", nameDatabaseConfig, err)
	}

	for _, k := range []string{"DATABASE_URL", "secret-config.toml"} {
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
		if kind, _ := sub["kind"].(string); kind == "ServiceAccount" {
			sub["namespace"] = namespace
			subjects[i] = sub
		}
	}
	_ = unstructured.SetNestedSlice(obj.Object, subjects, "subjects")
}

func setConfigMapAudiences(resources []*unstructured.Unstructured, audiences []string) bool {
	const key = "token_review_audiences"
	for _, obj := range resources {
		if obj.GetKind() != kindConfigMap {
			continue
		}
		data, found, _ := unstructured.NestedStringMap(obj.Object, "data")
		if !found {
			continue
		}
		toml, ok := data["config.toml"]
		if !ok || !strings.Contains(toml, "[auth]") {
			continue
		}

		quoted := make([]string, len(audiences))
		for i, a := range audiences {
			quoted[i] = fmt.Sprintf("%q", a)
		}
		audienceLine := fmt.Sprintf("%s = [%s]", key, strings.Join(quoted, ", "))

		replaced := false
		var result []string
		for line := range strings.SplitSeq(toml, "\n") {
			trimmed := strings.TrimSpace(line)
			if strings.HasPrefix(trimmed, key) && (len(trimmed) == len(key) || trimmed[len(key)] == ' ' || trimmed[len(key)] == '=') {
				result = append(result, audienceLine)
				replaced = true
			} else {
				result = append(result, line)
			}
		}

		if !replaced {
			var inserted []string
			inAuth := false
			done := false
			for _, line := range result {
				trimmed := strings.TrimSpace(line)
				if trimmed == "[auth]" {
					inAuth = true
				}
				if inAuth && !done && trimmed != "[auth]" && (strings.HasPrefix(trimmed, "[") || trimmed == "") {
					inserted = append(inserted, audienceLine)
					done = true
				}
				inserted = append(inserted, line)
			}
			if inAuth && !done {
				inserted = append(inserted, audienceLine)
			}
			result = inserted
		}

		data["config.toml"] = strings.Join(result, "\n")
		_ = unstructured.SetNestedStringMap(obj.Object, data, "data")
		return true
	}
	return false
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
			if name != "kube-rbac-proxy" {
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

func annotateDeploymentWithConfigHash(resources []*unstructured.Unstructured, deploymentContainer, configMapSuffix string) {
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
				if name, _ := container["name"].(string); name == deploymentContainer {
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
		ann["dataconnecthub/config-hash"] = configHash
		_ = unstructured.SetNestedStringMap(obj.Object, ann, "spec", "template", "metadata", "annotations")
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
