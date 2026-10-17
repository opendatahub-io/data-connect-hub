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
	"slices"
	"strings"

	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	apimachtypes "k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	logf "sigs.k8s.io/controller-runtime/pkg/log"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	// RelatedImageRestService is the env var for the REST image.
	RelatedImageRestService = "RELATED_IMAGE_ODH_DATA_CONNECT_HUB_REST_IMAGE"
	// RelatedImageFlightService is the env var for the Flight image.
	RelatedImageFlightService = "RELATED_IMAGE_ODH_DATA_CONNECT_HUB_FLIGHT_IMAGE"
	// RelatedImageKubeRbacProxy is the env var for the kube-rbac-proxy image.
	RelatedImageKubeRbacProxy = "RELATED_IMAGE_ODH_KUBE_RBAC_PROXY_IMAGE"
)

// --- Apply resources with SSA and owner references ---

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

	for _, k := range []string{nameSecretConfigTOML} {
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

// --- Content hash for rolling restarts ---

func (r *DataConnectServiceReconciler) annotateDeploymentsWithContentHash(
	ctx context.Context,
	resources []*unstructured.Unstructured,
	namespace string,
) error {
	for _, obj := range resources {
		if obj.GetKind() != kindDeployment {
			continue
		}
		hash, err := r.computeDeploymentContentHash(ctx, resources, namespace, obj)
		if err != nil {
			return fmt.Errorf("computing content hash for deployment %s: %w", obj.GetName(), err)
		}
		if hash == "" {
			continue
		}
		ann, _, _ := unstructured.NestedStringMap(obj.Object, "spec", "template", "metadata", "annotations")
		if ann == nil {
			ann = map[string]string{}
		}
		ann[annotationConfigHash] = hash
		_ = unstructured.SetNestedStringMap(obj.Object, ann, "spec", "template", "metadata", "annotations")
	}
	return nil
}

func (r *DataConnectServiceReconciler) computeDeploymentContentHash(
	ctx context.Context,
	resources []*unstructured.Unstructured,
	namespace string,
	deployment *unstructured.Unstructured,
) (string, error) {
	volumes, found, _ := unstructured.NestedSlice(deployment.Object, "spec", "template", "spec", "volumes")
	if !found {
		return "", nil
	}

	h := sha256.New()
	hasContent := false

	for _, v := range volumes {
		vol, ok := v.(map[string]any)
		if !ok {
			continue
		}

		if cm, ok := vol["configMap"].(map[string]any); ok {
			name, _ := cm["name"].(string)
			if name == "" {
				continue
			}
			data := findRenderedConfigMapData(resources, name)
			var binaryData map[string][]byte
			if data == nil {
				var liveCM corev1.ConfigMap
				if err := r.Get(ctx, client.ObjectKey{Namespace: namespace, Name: name}, &liveCM); err != nil {
					if apierrors.IsNotFound(err) {
						continue
					}
					return "", err
				}
				data = liveCM.Data
				binaryData = liveCM.BinaryData
			}
			if len(data) > 0 || len(binaryData) > 0 {
				h.Write([]byte("configmap:"))
				h.Write([]byte(name))
				h.Write([]byte{0})
				b, _ := json.Marshal(data)
				h.Write(b)
				bb, _ := json.Marshal(binaryData)
				h.Write(bb)
				hasContent = true
			}
		}

		if sec, ok := vol["secret"].(map[string]any); ok {
			name, _ := sec["secretName"].(string)
			if name == "" {
				continue
			}
			var liveSecret corev1.Secret
			if err := r.Get(ctx, client.ObjectKey{Namespace: namespace, Name: name}, &liveSecret); err != nil {
				if apierrors.IsNotFound(err) {
					continue
				}
				return "", err
			}
			if len(liveSecret.Data) > 0 {
				b, _ := json.Marshal(liveSecret.Data)
				h.Write([]byte("secret:"))
				h.Write([]byte(name))
				h.Write([]byte{0})
				h.Write(b)
				hasContent = true
			}
		}
	}

	if !hasContent {
		return "", nil
	}
	return hex.EncodeToString(h.Sum(nil))[:16], nil
}

func findRenderedConfigMapData(resources []*unstructured.Unstructured, name string) map[string]string {
	for _, obj := range resources {
		if obj.GetKind() == kindConfigMap && obj.GetName() == name {
			data, found, _ := unstructured.NestedStringMap(obj.Object, "data")
			if found && len(data) > 0 {
				return data
			}
			return nil
		}
	}
	return nil
}
