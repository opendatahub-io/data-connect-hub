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

	apiequality "k8s.io/apimachinery/pkg/api/equality"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	labelGatewayOwnerUID = "dataconnecthub.opendatahub.io/owner-uid"
	gatewaySpecField     = "spec"
	gatewayKindField     = "kind"
)

var httpRouteGVK = schema.GroupVersionKind{Group: gatewayAPIGroup, Version: "v1", Kind: kindHTTPRoute}
var referenceGrantGVK = schema.GroupVersionKind{Group: gatewayAPIGroup, Version: "v1beta1", Kind: "ReferenceGrant"}

// reconcileGatewayRouting keeps HTTPRoute placement independent of operand
// placement. Cross-namespace routes cannot have an ownerReference to the CR;
// a UID label and the CR finalizer provide explicit ownership and cleanup.
func (r *DataConnectServiceReconciler) reconcileGatewayRouting(
	ctx context.Context, cr *dchv1alpha1.DataConnectService, gw dchv1alpha1.Gateway,
	resources []*unstructured.Unstructured,
) ([]*unstructured.Unstructured, error) {
	if gw.RouteNamespace == "" || gw.RouteNamespace == cr.Namespace {
		return resources, r.deleteGatewayGrants(ctx, cr)
	}
	remaining := make([]*unstructured.Unstructured, 0, len(resources))
	for _, obj := range resources {
		if obj.GetKind() != kindHTTPRoute {
			remaining = append(remaining, obj)
			continue
		}
		obj.SetNamespace(gw.RouteNamespace)
		rules, _, err := unstructured.NestedSlice(obj.Object, gatewaySpecField, "rules")
		if err != nil {
			return nil, err
		}
		var grants []*unstructured.Unstructured
		for i, item := range rules {
			rule, ok := item.(map[string]any)
			if !ok {
				return nil, fmt.Errorf("invalid HTTPRoute rule %d", i)
			}
			backends, _, err := unstructured.NestedSlice(rule, "backendRefs")
			if err != nil {
				return nil, err
			}
			for j, item := range backends {
				backend, ok := item.(map[string]any)
				if !ok {
					return nil, fmt.Errorf("invalid HTTPRoute backend %d/%d", i, j)
				}
				backend["namespace"] = cr.Namespace
				name, _ := backend[resourceNameField].(string)
				// Keep one named Service per grant for Gateway compatibility.
				grant := &unstructured.Unstructured{Object: map[string]any{
					"apiVersion":     referenceGrantGVK.GroupVersion().String(),
					gatewayKindField: referenceGrantGVK.Kind,
					gatewaySpecField: map[string]any{
						"from": []any{map[string]any{"group": httpRouteGVK.Group, gatewayKindField: kindHTTPRoute, "namespace": gw.RouteNamespace}},
						"to":   []any{map[string]any{"group": "", gatewayKindField: kindService, resourceNameField: name}},
					},
				}}
				grant.SetName(fmt.Sprintf("%s-backend-%d-%d", cr.Name, i, j))
				current := &unstructured.Unstructured{}
				current.SetGroupVersionKind(referenceGrantGVK)
				key := client.ObjectKey{Name: grant.GetName(), Namespace: cr.Namespace}
				if err := r.Get(ctx, key, current); err == nil {
					if !hasControllerOwner(current, cr.UID) {
						return nil, fmt.Errorf("ReferenceGrant %s is not owned by this DataConnectService", key)
					}
				} else if !apierrors.IsNotFound(err) {
					return nil, err
				}
				grants = append(grants, grant)
			}
			if err := unstructured.SetNestedSlice(rule, backends, "backendRefs"); err != nil {
				return nil, err
			}
		}
		if err := unstructured.SetNestedSlice(obj.Object, rules, gatewaySpecField, "rules"); err != nil {
			return nil, err
		}
		if err := r.applyResources(ctx, cr, cr.Namespace, grants); err != nil {
			return nil, fmt.Errorf("applying cross-namespace backend grants: %w", err)
		}
		if err := r.applyCrossNamespaceRoute(ctx, cr, obj); err != nil {
			return nil, err
		}
	}
	return remaining, nil
}

func (r *DataConnectServiceReconciler) applyCrossNamespaceRoute(ctx context.Context, cr *dchv1alpha1.DataConnectService, desired *unstructured.Unstructured) error {
	current := &unstructured.Unstructured{}
	current.SetGroupVersionKind(httpRouteGVK)
	err := r.Get(ctx, client.ObjectKeyFromObject(desired), current)
	if err != nil && !apierrors.IsNotFound(err) {
		return fmt.Errorf("reading application HTTPRoute: %w", err)
	}
	if err == nil && current.GetLabels()[labelGatewayOwnerUID] != string(cr.UID) {
		return fmt.Errorf("HTTPRoute %s/%s is not owned by DataConnectService %s/%s", desired.GetNamespace(), desired.GetName(), cr.Namespace, cr.Name)
	}
	labels := desired.GetLabels()
	if labels == nil {
		labels = map[string]string{}
	}
	labels[labelManagedBy] = managedByDCHService
	labels[labelGatewayOwnerUID] = string(cr.UID)
	desired.SetLabels(labels)
	desired.SetOwnerReferences(nil)
	if apierrors.IsNotFound(err) {
		return r.Create(ctx, desired)
	}
	if apiequality.Semantic.DeepDerivative(desired.Object[gatewaySpecField], current.Object[gatewaySpecField]) {
		return nil
	}
	current.Object[gatewaySpecField] = desired.Object[gatewaySpecField]
	current.SetLabels(labels)
	return r.Update(ctx, current)
}

func (r *DataConnectServiceReconciler) deleteGatewayRoutes(ctx context.Context, cr *dchv1alpha1.DataConnectService, keepNamespace string) error {
	list := &unstructured.UnstructuredList{}
	list.SetGroupVersionKind(httpRouteGVK.GroupVersion().WithKind("HTTPRouteList"))
	if err := r.List(ctx, list, client.MatchingLabels{labelManagedBy: managedByDCHService}); err != nil {
		if meta.IsNoMatchError(err) || apierrors.IsNotFound(err) {
			return nil
		}
		return fmt.Errorf("listing DCH HTTPRoutes for cleanup: %w", err)
	}
	for i := range list.Items {
		obj := &list.Items[i]
		owned := cr.UID != "" && obj.GetLabels()[labelGatewayOwnerUID] == string(cr.UID) || hasControllerOwner(obj, cr.UID)
		if !owned || keepNamespace != "" && obj.GetNamespace() == keepNamespace && obj.GetName() == httpRouteResourceName(cr.Name) {
			continue
		}
		uid := obj.GetUID()
		if err := r.Delete(ctx, obj, &client.DeleteOptions{Preconditions: &metav1.Preconditions{UID: &uid}}); client.IgnoreNotFound(err) != nil {
			return fmt.Errorf("deleting DCH HTTPRoute: %w", err)
		}
		if keepNamespace == "" {
			current := &unstructured.Unstructured{}
			current.SetGroupVersionKind(httpRouteGVK)
			if err := r.Get(ctx, client.ObjectKeyFromObject(obj), current); err == nil {
				return fmt.Errorf("waiting for HTTPRoute %s/%s cleanup", obj.GetNamespace(), obj.GetName())
			} else if !apierrors.IsNotFound(err) {
				return err
			}
		}
	}
	return nil
}

func (r *DataConnectServiceReconciler) deleteGatewayGrants(ctx context.Context, cr *dchv1alpha1.DataConnectService) error {
	list := &unstructured.UnstructuredList{}
	list.SetGroupVersionKind(referenceGrantGVK.GroupVersion().WithKind("ReferenceGrantList"))
	if err := r.List(ctx, list, client.InNamespace(cr.Namespace), client.MatchingLabels{labelManagedBy: managedByDCHService}); err != nil {
		if meta.IsNoMatchError(err) || apierrors.IsNotFound(err) {
			return nil
		}
		return fmt.Errorf("listing DCH ReferenceGrants: %w", err)
	}
	for i := range list.Items {
		obj := &list.Items[i]
		if hasControllerOwner(obj, cr.UID) {
			uid := obj.GetUID()
			if err := r.Delete(ctx, obj, &client.DeleteOptions{Preconditions: &metav1.Preconditions{UID: &uid}}); client.IgnoreNotFound(err) != nil {
				return err
			}
		}
	}
	return nil
}
