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
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/kustomize/api/krusty"
	kustypes "sigs.k8s.io/kustomize/api/types"
	"sigs.k8s.io/kustomize/kyaml/filesys"
	"sigs.k8s.io/kustomize/kyaml/resid"
	sigyaml "sigs.k8s.io/yaml"

	dataconnecthubv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/v1alpha1"
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

func renderPostgresKustomization(diskPath string) ([]*unstructured.Unstructured, error) {
	absPath, err := filepath.Abs(diskPath)
	if err != nil {
		return nil, fmt.Errorf("resolving path %s: %w", diskPath, err)
	}

	memFS := filesys.MakeFsInMemory()
	if err := copyDirToMemFS(absPath, memFS); err != nil {
		return nil, fmt.Errorf("copying postgres manifests to memory: %w", err)
	}

	if err := stripSecretGenerator(memFS, absPath); err != nil {
		return nil, fmt.Errorf("stripping secret generator: %w", err)
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

func stripSecretGenerator(fs filesys.FileSystem, dir string) error {
	kustPath := filepath.Join(dir, "kustomization.yaml")
	data, err := fs.ReadFile(kustPath)
	if err != nil {
		return fmt.Errorf("reading kustomization: %w", err)
	}

	var kust map[string]any
	if err := sigyaml.Unmarshal(data, &kust); err != nil {
		return fmt.Errorf("parsing kustomization: %w", err)
	}

	delete(kust, "secretGenerator")
	delete(kust, "generatorOptions")

	out, err := sigyaml.Marshal(kust)
	if err != nil {
		return fmt.Errorf("serializing kustomization: %w", err)
	}
	return fs.WriteFile(kustPath, out)
}

// --- CR overrides → kustomize patches ---

func buildServicePatches(name string, overrides *dataconnecthubv1alpha1.ServiceOverrides) []kustypes.Patch {
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
					Gvk:  resid.Gvk{Group: "apps", Version: "v1", Kind: "Deployment"},
					Name: name,
				},
			},
			Patch: fmt.Sprintf("apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: %s\n%s", name, part),
		})
	}

	return patches
}

func resolveServiceImage(name string, overrides *dataconnecthubv1alpha1.ServiceOverrides, restImage, flightImage string) string {
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
		if obj.GetKind() != "Deployment" {
			continue
		}
		containers, found, _ := unstructured.NestedSlice(obj.Object, "spec", "template", "spec", "containers")
		if !found {
			continue
		}
		for i, c := range containers {
			container, ok := c.(map[string]interface{})
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

func buildGatewayPatches(gw *dataconnecthubv1alpha1.Gateway) []kustypes.Patch {
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

func (r *DataConnectHubReconciler) applyResources(
	ctx context.Context,
	cr *dataconnecthubv1alpha1.DataConnectHub,
	resources []*unstructured.Unstructured,
) error {
	log := logf.FromContext(ctx)

	for _, obj := range resources {
		obj.SetNamespace(r.Namespace)

		labels := obj.GetLabels()
		if labels == nil {
			labels = map[string]string{}
		}
		labels["components.platform.opendatahub.io/managed-by"] = "dataconnecthub"
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

		if obj.GetKind() == "PersistentVolumeClaim" {
			continue
		}

		existingHash := ""
		if existingAnn := existing.GetAnnotations(); existingAnn != nil {
			existingHash = existingAnn["dataconnecthub/spec-hash"]
		}
		if existingHash == desiredHash {
			if existing.GetOwnerReferences() == nil || len(existing.GetOwnerReferences()) == 0 {
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

// --- Postgres secret (programmatic — random password generation) ---

func (r *DataConnectHubReconciler) reconcilePostgresSecret(ctx context.Context, cr *dataconnecthubv1alpha1.DataConnectHub) error {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      namePostgresCreds,
			Namespace: r.Namespace,
		},
	}

	mutateFn := func() error {
		if secret.Labels == nil {
			secret.Labels = map[string]string{}
		}
		secret.Labels["app.kubernetes.io/name"] = namePostgres
		secret.Labels["app.kubernetes.io/part-of"] = nameDataConnectHub

		requiredKeys := []string{"POSTGRESQL_USER", "POSTGRESQL_PASSWORD", "POSTGRESQL_DATABASE", "secret-config.toml"}
		hasAllKeys := len(secret.Data) >= len(requiredKeys)
		for _, k := range requiredKeys {
			if _, ok := secret.Data[k]; !ok {
				hasAllKeys = false
				break
			}
		}
		if !hasAllKeys {
			password := generatePassword(24)
			dbUser := "dch"
			dbName := "dataconnecthub"
			connURL := fmt.Sprintf("postgresql://%s:%s@postgres:5432/%s", dbUser, password, dbName)

			secret.StringData = map[string]string{
				"POSTGRESQL_USER":     dbUser,
				"POSTGRESQL_PASSWORD": password,
				"POSTGRESQL_DATABASE": dbName,
			}
			secret.Data = map[string][]byte{
				"secret-config.toml": fmt.Appendf(nil, "[database]\nurl = \"%s\"\n", connURL),
			}
		}

		return controllerutil.SetControllerReference(cr, secret, r.Scheme)
	}

	_, err := controllerutil.CreateOrUpdate(ctx, r.Client, secret, mutateFn)
	if apierrors.IsAlreadyExists(err) {
		// Cache was stale — retry now that the informer has caught up
		_, err = controllerutil.CreateOrUpdate(ctx, r.Client, secret, mutateFn)
	}
	return err
}

// --- Helpers ---

func generatePassword(length int) string {
	b := make([]byte, length)
	_, _ = rand.Read(b)
	return hex.EncodeToString(b)[:length]
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
