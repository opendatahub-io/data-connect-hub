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
	"reflect"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/builder"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/event"
	"sigs.k8s.io/controller-runtime/pkg/handler"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/predicate"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	// labelTenantNamespace marks a namespace as a DCH tenant. The controller
	// reconciles the REST service's secret permissions into labelled
	// namespaces and removes them when the label is removed.
	labelTenantNamespace = "dataconnecthub.opendatahub.io/tenant"

	// kustomizeNamePrefix is the namePrefix applied to every resource
	// rendered from config/base/kustomization.yaml. TestTenantRBACNames
	// pins the rendered names to the constants below.
	kustomizeNamePrefix = "dch-"

	// nameRestServiceSecretAccess is the un-prefixed name shared by the
	// ClusterRole granting the REST service access to connection secrets
	// and by the tenant-local RoleBinding that binds it.
	nameRestServiceSecretAccess = "rest-service-secret-access"
)

// tenantSecretAccessClusterRoleName returns the name of the rendered
// ClusterRole granting the REST service access to connection secrets.
func tenantSecretAccessClusterRoleName() string {
	return kustomizeNamePrefix + nameRestServiceSecretAccess
}

// tenantSecretAccessRoleBindingName returns the name of the tenant-local
// RoleBinding that binds the secret-access ClusterRole to the REST
// ServiceAccount.
func tenantSecretAccessRoleBindingName() string {
	return kustomizeNamePrefix + nameRestServiceSecretAccess
}

// restServiceAccountName returns the name of the rendered REST ServiceAccount.
func restServiceAccountName() string {
	return kustomizeNamePrefix + nameRestService + "-sa"
}

// TenantNamespaceReconciler watches namespaces labelled as DCH tenants and
// binds the rest-service-secret-access ClusterRole to the REST ServiceAccount
// in each of them, so connection creation works in newly created projects
// without a manual per-namespace RBAC step. Bindings carry the managed-by
// label instead of an owner reference: the DataConnectService CR is
// namespaced and cannot own objects in tenant namespaces.
type TenantNamespaceReconciler struct {
	client.Client
	Scheme *runtime.Scheme
}

// +kubebuilder:rbac:groups="",resources=namespaces,verbs=get;list;watch
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=rolebindings,verbs=get;list;watch;create;update;patch;delete

// Reconcile ensures the tenant-local RoleBinding matches the namespace's
// tenant label and the live DataConnectService instance.
func (r *TenantNamespaceReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	var ns corev1.Namespace
	if err := r.Get(ctx, client.ObjectKey{Name: req.Name}, &ns); err != nil {
		if apierrors.IsNotFound(err) {
			return ctrl.Result{}, nil
		}
		return ctrl.Result{}, err
	}

	cr, found, err := firstDataConnectService(ctx, r.Client)
	if err != nil {
		return ctrl.Result{}, err
	}
	if !found {
		// No live DataConnectService: nothing to grant for. The finalizer
		// removes bindings the controller created earlier.
		return ctrl.Result{}, nil
	}

	if !isTenantNamespace(&ns) {
		return ctrl.Result{}, r.removeRoleBinding(ctx, ns.Name)
	}
	return ctrl.Result{}, r.ensureRoleBinding(ctx, cr, ns.Name)
}

// ensureRoleBinding creates or repairs the secret-access RoleBinding in the
// tenant namespace. An existing binding with a different roleRef is replaced,
// since roleRef is immutable.
func (r *TenantNamespaceReconciler) ensureRoleBinding(ctx context.Context, cr *dchv1alpha1.DataConnectService, namespace string) error {
	log := logf.FromContext(ctx)

	desired := &rbacv1.RoleBinding{
		ObjectMeta: metav1.ObjectMeta{
			Name:      tenantSecretAccessRoleBindingName(),
			Namespace: namespace,
			Labels:    map[string]string{labelManagedBy: managedByDCHService},
		},
		RoleRef: rbacv1.RoleRef{
			APIGroup: rbacv1.GroupName,
			Kind:     kindClusterRole,
			Name:     tenantSecretAccessClusterRoleName(),
		},
		Subjects: []rbacv1.Subject{{
			Kind:      rbacv1.ServiceAccountKind,
			Name:      restServiceAccountName(),
			Namespace: cr.Namespace,
		}},
	}

	existing := &rbacv1.RoleBinding{}
	err := r.Get(ctx, client.ObjectKeyFromObject(desired), existing)
	if apierrors.IsNotFound(err) {
		if err := r.Create(ctx, desired); err != nil && !apierrors.IsAlreadyExists(err) {
			return fmt.Errorf("creating tenant RoleBinding %s/%s: %w", namespace, desired.Name, err)
		}
		log.Info("Created tenant RoleBinding", "name", desired.Name, "namespace", namespace)
		return nil
	}
	if err != nil {
		return fmt.Errorf("getting tenant RoleBinding %s/%s: %w", namespace, desired.Name, err)
	}

	if existing.RoleRef != desired.RoleRef {
		if err := r.Delete(ctx, existing); err != nil && !apierrors.IsNotFound(err) {
			return fmt.Errorf("deleting drifted tenant RoleBinding %s/%s: %w", namespace, existing.Name, err)
		}
		if err := r.Create(ctx, desired); err != nil && !apierrors.IsAlreadyExists(err) {
			return fmt.Errorf("recreating tenant RoleBinding %s/%s: %w", namespace, desired.Name, err)
		}
		log.Info("Replaced drifted tenant RoleBinding", "name", desired.Name, "namespace", namespace)
		return nil
	}

	changed := false
	if existing.Labels[labelManagedBy] != managedByDCHService {
		labels := existing.Labels
		if labels == nil {
			labels = make(map[string]string)
		}
		labels[labelManagedBy] = managedByDCHService
		existing.Labels = labels
		changed = true
	}
	if !reflect.DeepEqual(existing.Subjects, desired.Subjects) {
		existing.Subjects = desired.Subjects
		changed = true
	}
	if changed {
		if err := r.Update(ctx, existing); err != nil {
			return fmt.Errorf("updating tenant RoleBinding %s/%s: %w", namespace, existing.Name, err)
		}
		log.Info("Updated tenant RoleBinding", "name", existing.Name, "namespace", namespace)
	}
	return nil
}

// removeRoleBinding deletes the controller-managed binding when a namespace
// stops being a tenant. Bindings without the managed-by label — for example
// the manual RoleBinding documented for standalone installs — are left alone.
func (r *TenantNamespaceReconciler) removeRoleBinding(ctx context.Context, namespace string) error {
	binding := &rbacv1.RoleBinding{
		ObjectMeta: metav1.ObjectMeta{
			Name:      tenantSecretAccessRoleBindingName(),
			Namespace: namespace,
		},
	}
	if err := r.Get(ctx, client.ObjectKeyFromObject(binding), binding); err != nil {
		if apierrors.IsNotFound(err) {
			return nil
		}
		return fmt.Errorf("getting tenant RoleBinding %s/%s: %w", namespace, binding.Name, err)
	}
	if binding.Labels[labelManagedBy] != managedByDCHService {
		return nil
	}
	if err := r.Delete(ctx, binding); err != nil && !apierrors.IsNotFound(err) {
		return fmt.Errorf("deleting tenant RoleBinding %s/%s: %w", namespace, binding.Name, err)
	}
	logf.FromContext(ctx).Info("Deleted tenant RoleBinding", "name", binding.Name, "namespace", namespace)
	return nil
}

// deleteTenantRoleBindings removes the tenant-local RoleBindings created by
// the tenant-namespace reconciler. They carry the managed-by label instead
// of an owner reference — the DataConnectService CR is namespaced and cannot
// own objects in tenant namespaces — so uninstall cleans them up by label
// and name.
func (r *DataConnectServiceReconciler) deleteTenantRoleBindings(ctx context.Context) error {
	log := logf.FromContext(ctx)

	var bindings rbacv1.RoleBindingList
	if err := r.List(ctx, &bindings, client.MatchingLabels{labelManagedBy: managedByDCHService}); err != nil {
		return fmt.Errorf("listing DCH RoleBindings for cleanup: %w", err)
	}

	var cleanupErr error
	for i := range bindings.Items {
		binding := &bindings.Items[i]
		if binding.Name != tenantSecretAccessRoleBindingName() {
			continue
		}
		if err := r.Delete(ctx, binding); err != nil && !apierrors.IsNotFound(err) {
			log.Error(err, "Failed to delete tenant RoleBinding", "name", binding.Name, "namespace", binding.Namespace)
			cleanupErr = errors.Join(cleanupErr, fmt.Errorf("deleting RoleBinding %s/%s: %w", binding.Namespace, binding.Name, err))
		}
	}
	return cleanupErr
}

// firstDataConnectService returns the first DataConnectService that is not
// being deleted. DCH runs one service instance per cluster; the tenant
// reconciler only grants permissions while a live instance exists.
func firstDataConnectService(ctx context.Context, reader client.Reader) (*dchv1alpha1.DataConnectService, bool, error) {
	var list dchv1alpha1.DataConnectServiceList
	if err := reader.List(ctx, &list); err != nil {
		if apierrors.IsNotFound(err) {
			return nil, false, nil
		}
		return nil, false, err
	}
	for i := range list.Items {
		if list.Items[i].DeletionTimestamp.IsZero() {
			return &list.Items[i], true, nil
		}
	}
	return nil, false, nil
}

// isTenantNamespace reports whether the object carries the DCH tenant label.
func isTenantNamespace(obj metav1.Object) bool {
	return obj.GetLabels()[labelTenantNamespace] == valueTrue
}

// tenantNamespacesToRequests enqueues every labelled namespace so a newly
// created DataConnectService reconciles bindings for namespaces labelled
// before the service existed.
func (r *TenantNamespaceReconciler) tenantNamespacesToRequests(ctx context.Context, _ client.Object) []reconcile.Request {
	var list corev1.NamespaceList
	if err := r.List(ctx, &list, client.MatchingLabels{labelTenantNamespace: valueTrue}); err != nil {
		logf.FromContext(ctx).Error(err, "failed to list tenant namespaces for DataConnectService trigger")
		return nil
	}
	requests := make([]reconcile.Request, 0, len(list.Items))
	for i := range list.Items {
		requests = append(requests, reconcile.Request{
			NamespacedName: types.NamespacedName{Name: list.Items[i].Name},
		})
	}
	return requests
}

// tenantNamespacePredicate passes namespace events where the tenant label is
// present, and update events where it was present on the old object so label
// removal also triggers cleanup.
func tenantNamespacePredicate() predicate.Predicate {
	return predicate.Funcs{
		CreateFunc: func(e event.CreateEvent) bool { return isTenantNamespace(e.Object) },
		UpdateFunc: func(e event.UpdateEvent) bool {
			return isTenantNamespace(e.ObjectNew) || isTenantNamespace(e.ObjectOld)
		},
		DeleteFunc:  func(e event.DeleteEvent) bool { return isTenantNamespace(e.Object) },
		GenericFunc: func(e event.GenericEvent) bool { return isTenantNamespace(e.Object) },
	}
}

// SetupWithManager sets up the controller with the Manager.
func (r *TenantNamespaceReconciler) SetupWithManager(mgr ctrl.Manager) error {
	return ctrl.NewControllerManagedBy(mgr).
		For(&corev1.Namespace{}, builder.WithPredicates(tenantNamespacePredicate())).
		Watches(
			&dchv1alpha1.DataConnectService{},
			handler.EnqueueRequestsFromMapFunc(r.tenantNamespacesToRequests),
		).
		Named("tenantnamespace").
		Complete(r)
}
