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

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/builder"
	"sigs.k8s.io/controller-runtime/pkg/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/event"
	"sigs.k8s.io/controller-runtime/pkg/handler"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/predicate"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"
	"sigs.k8s.io/controller-runtime/pkg/recorder"
	"sigs.k8s.io/controller-runtime/pkg/source"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	// labelTenantNamespace marks a namespace as a DCH tenant. The controller
	// reconciles the REST service's secret permissions into labelled
	// namespaces and removes them when the label is removed.
	labelTenantNamespace = "dataconnecthub.opendatahub.io/tenant"

	// kustomizeNamePrefix is the namePrefix applied to every resource
	// rendered from the data-connect-hub chart. TestTenantRBACNames
	// pins the rendered names to the constants below.
	kustomizeNamePrefix = "dch-"

	// nameRestServiceSecretAccess is the un-prefixed name shared by the
	// ClusterRole granting the REST service access to connection secrets
	// and by the tenant-local RoleBinding that binds it.
	nameRestServiceSecretAccess = "rest-service-secret-access"
	tenantNSFieldOwner          = "dc-controller-tenant-namespace"
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
// without a manual per-namespace RBAC step. It also watches its own managed
// bindings so state hidden from namespace events still converges: a label
// removed while the controller was down, or a binding deleted or drifted
// while the label stayed. Bindings carry the managed-by label instead of an
// owner reference: the DataConnectService CR is namespaced and cannot own
// objects in tenant namespaces.
type TenantNamespaceReconciler struct {
	client.Client
	Scheme   *runtime.Scheme
	Recorder recorder.EventRecorder

	// APIReader reads directly from the API server. The reconciler decides
	// whether to create or remove bindings from the informer cache, which
	// can lag behind a DataConnectService deletion; APIReader provides the
	// authoritative reads that keep a binding from outliving the service
	// it grants for.
	APIReader client.Reader
}

// +kubebuilder:rbac:groups="",resources=namespaces,verbs=get;list;watch
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=rolebindings,verbs=get;list;watch;create
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=rolebindings,resourceNames=dch-rest-service-secret-access,verbs=update;patch;delete
// +kubebuilder:rbac:groups=events.k8s.io,resources=events,verbs=create;patch

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
		// No live DataConnectService: nothing to grant for. Remove any
		// leftover managed binding — a reconcile that read a live service
		// from a stale cache can create one after the finalizer's
		// uninstall sweep has finished, and the DataConnectService delete
		// event requeues every tenant namespace to run this cleanup.
		return ctrl.Result{}, r.removeRoleBinding(ctx, ns.Name)
	}

	if !isTenantNamespace(&ns) {
		return ctrl.Result{}, r.removeRoleBinding(ctx, ns.Name)
	}
	return ctrl.Result{}, r.ensureRoleBinding(ctx, cr, &ns)
}

// ensureRoleBinding creates or repairs the secret-access RoleBinding in the
// tenant namespace with server-side apply. A managed binding with a different
// roleRef is replaced, since roleRef is immutable; while a finalizer keeps
// the old binding terminating, the replacement requeues instead of reporting
// success. Existing bindings are checked only for the managed-by label before
// apply; no existing fields are compared. Apply conflicts are also reported
// as unmanaged bindings rather than taking ownership. Every applied binding
// is verified against the API server so a binding applied while the service
// is being removed is cleaned up instead of outliving the service.
func (r *TenantNamespaceReconciler) ensureRoleBinding(ctx context.Context, cr *dchv1alpha1.DataConnectService, ns *corev1.Namespace) error {
	log := logf.FromContext(ctx)
	namespace := ns.Name

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
	if err := r.APIReader.Get(ctx, client.ObjectKeyFromObject(desired), existing); err == nil {
		if existing.Labels[labelManagedBy] != managedByDCHService {
			return r.reportUnmanagedConflict(ns, existing)
		}
	} else if !apierrors.IsNotFound(err) {
		return fmt.Errorf("checking tenant RoleBinding %s/%s: %w", namespace, desired.Name, err)
	}

	err := r.applyTenantRoleBinding(ctx, ns, desired)
	if err != nil {
		return err
	}

	if err := r.removeIfServiceRemoved(ctx, desired); err != nil {
		return err
	}
	log.Info("Applied tenant RoleBinding", "name", desired.Name, "namespace", namespace)
	return nil
}

// applyTenantRoleBinding applies the desired binding without taking ownership
// of an administrator's binding. A conflict on a binding already marked as
// managed is repaired with force ownership, which lets the controller restore
// drift without adopting an unrelated binding.
func (r *TenantNamespaceReconciler) applyTenantRoleBinding(ctx context.Context, ns *corev1.Namespace, desired *rbacv1.RoleBinding) error {
	err := r.Apply(ctx, roleBindingApplyConfig(desired), client.FieldOwner(tenantNSFieldOwner))
	if apierrors.IsConflict(err) {
		existing := &rbacv1.RoleBinding{}
		if getErr := r.APIReader.Get(ctx, client.ObjectKeyFromObject(desired), existing); getErr != nil {
			return fmt.Errorf("getting tenant RoleBinding %s/%s after apply conflict: %w", desired.Namespace, desired.Name, getErr)
		}
		if existing.Labels[labelManagedBy] != managedByDCHService {
			return r.reportUnmanagedConflict(ns, existing)
		}
		err = r.Apply(ctx, roleBindingApplyConfig(desired), client.FieldOwner(tenantNSFieldOwner), client.ForceOwnership)
	}
	if isRoleRefChange(err) {
		stale := &rbacv1.RoleBinding{ObjectMeta: metav1.ObjectMeta{Namespace: desired.Namespace, Name: desired.Name}}
		if deleteErr := r.Delete(ctx, stale); deleteErr != nil && !apierrors.IsNotFound(deleteErr) {
			return fmt.Errorf("deleting drifted tenant RoleBinding %s/%s: %w", desired.Namespace, desired.Name, deleteErr)
		}
		err = r.Apply(ctx, roleBindingApplyConfig(desired), client.FieldOwner(tenantNSFieldOwner), client.ForceOwnership)
		if apierrors.IsAlreadyExists(err) || apierrors.IsInvalid(err) {
			return fmt.Errorf("tenant RoleBinding %s/%s is still terminating, requeuing replacement", desired.Namespace, desired.Name)
		}
	}
	if err != nil {
		return fmt.Errorf("applying tenant RoleBinding %s/%s: %w", desired.Namespace, desired.Name, err)
	}
	return nil
}

// reportUnmanagedConflict records the event and error for a binding the
// controller does not manage occupying the tenant binding name.
func (r *TenantNamespaceReconciler) reportUnmanagedConflict(ns *corev1.Namespace, existing *rbacv1.RoleBinding) error {
	r.Recorder.Eventf(ns, nil, corev1.EventTypeWarning, "UnmanagedRoleBindingConflict", "TenantOnboarding",
		"RoleBinding %s/%s was not created by the controller and was left unchanged; label it %s=%s or delete it to let the controller manage tenant access",
		existing.Namespace, existing.Name, labelManagedBy, managedByDCHService)
	return fmt.Errorf("unmanaged RoleBinding %s/%s conflicts with tenant onboarding", existing.Namespace, existing.Name)
}

// removeIfServiceRemoved removes the managed binding when no live
// DataConnectService remains. The decision to apply comes from the informer
// cache, which can lag behind deletion; the API reader prevents the applied
// binding from outliving the service it grants for.
func (r *TenantNamespaceReconciler) removeIfServiceRemoved(ctx context.Context, applied *rbacv1.RoleBinding) error {
	_, found, err := firstDataConnectService(ctx, r.APIReader)
	if err != nil {
		return fmt.Errorf("verifying the DataConnectService after applying tenant RoleBinding %s/%s: %w", applied.Namespace, applied.Name, err)
	}
	if found {
		return nil
	}
	if err := r.removeRoleBinding(ctx, applied.Namespace); err != nil {
		return err
	}
	return nil
}

// removeRoleBinding deletes the controller-managed binding when a namespace
// stops being a tenant or no live DataConnectService remains. Bindings
// without the managed-by label — for example the manual RoleBinding
// documented for standalone installs — are left alone.
func (r *TenantNamespaceReconciler) removeRoleBinding(ctx context.Context, namespace string) error {
	binding := &rbacv1.RoleBinding{
		ObjectMeta: metav1.ObjectMeta{
			Name:      tenantSecretAccessRoleBindingName(),
			Namespace: namespace,
		},
	}
	// Read through the API reader: a binding created moments ago may not
	// have reached the informer cache yet, and this cleanup path must not
	// miss it.
	if err := r.APIReader.Get(ctx, client.ObjectKeyFromObject(binding), binding); err != nil {
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
// and name. The list reads through the API reader rather than the informer
// cache: a binding a tenant reconcile committed moments ago may not have
// reached the cache yet, and once the finalizer clears no later sweep is
// guaranteed to remove it.
func (r *DataConnectServiceReconciler) deleteTenantRoleBindings(ctx context.Context) error {
	log := logf.FromContext(ctx)

	var bindings rbacv1.RoleBindingList
	if err := r.APIReader.List(ctx, &bindings, client.MatchingLabels{labelManagedBy: managedByDCHService}); err != nil {
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

// SetupWithManager sets up the controller with the Manager. RoleBindings
// are watched through a dedicated cache limited to managed bindings so the
// manager never caches every RoleBinding in the cluster.
func (r *TenantNamespaceReconciler) SetupWithManager(mgr ctrl.Manager) error {
	rbCache, err := cache.New(mgr.GetConfig(), cache.Options{
		Scheme: mgr.GetScheme(),
		Mapper: mgr.GetRESTMapper(),
		ByObject: map[client.Object]cache.ByObject{
			&rbacv1.RoleBinding{}: {Label: labels.SelectorFromSet(labels.Set{labelManagedBy: managedByDCHService})},
		},
	})
	if err != nil {
		return fmt.Errorf("creating RoleBinding cache: %w", err)
	}
	if err := mgr.Add(rbCache); err != nil {
		return fmt.Errorf("adding RoleBinding cache: %w", err)
	}

	return ctrl.NewControllerManagedBy(mgr).
		For(&corev1.Namespace{}, builder.WithPredicates(tenantNamespacePredicate())).
		Watches(
			&dchv1alpha1.DataConnectService{},
			handler.EnqueueRequestsFromMapFunc(r.tenantNamespacesToRequests),
		).
		WatchesRawSource(source.Kind(rbCache, &rbacv1.RoleBinding{},
			handler.TypedEnqueueRequestsFromMapFunc(roleBindingToNamespaceRequest),
			managedRoleBindingPredicate(),
		)).
		Named("tenantnamespace").
		Complete(r)
}

// roleBindingToNamespaceRequest enqueues the Namespace containing a managed
// RoleBinding, letting reconcile decide from the namespace's current labels
// whether the binding should still exist.
func roleBindingToNamespaceRequest(_ context.Context, obj *rbacv1.RoleBinding) []reconcile.Request {
	return []reconcile.Request{{NamespacedName: types.NamespacedName{Name: obj.Namespace}}}
}

// managedRoleBindingPredicate passes RoleBinding events for bindings this
// controller manages: the secret-access name carrying the managed-by label.
// Watching its own bindings lets the controller converge states no namespace
// event reveals — a tenant label removed while the controller was down
// leaves the binding in place with no event to retry, and a binding deleted
// or drifted while the label stayed likewise only shows up here. Events
// arrive from the label-scoped managed-bindings cache; the name check keeps
// bindings that merely share the managed-by label from requeuing tenants.
func managedRoleBindingPredicate() predicate.TypedPredicate[*rbacv1.RoleBinding] {
	isManaged := func(obj *rbacv1.RoleBinding) bool {
		return obj.Name == tenantSecretAccessRoleBindingName() &&
			obj.Labels[labelManagedBy] == managedByDCHService
	}
	return predicate.TypedFuncs[*rbacv1.RoleBinding]{
		CreateFunc: func(e event.TypedCreateEvent[*rbacv1.RoleBinding]) bool { return isManaged(e.Object) },
		UpdateFunc: func(e event.TypedUpdateEvent[*rbacv1.RoleBinding]) bool {
			return isManaged(e.ObjectNew) || isManaged(e.ObjectOld)
		},
		DeleteFunc:  func(e event.TypedDeleteEvent[*rbacv1.RoleBinding]) bool { return isManaged(e.Object) },
		GenericFunc: func(e event.TypedGenericEvent[*rbacv1.RoleBinding]) bool { return isManaged(e.Object) },
	}
}
