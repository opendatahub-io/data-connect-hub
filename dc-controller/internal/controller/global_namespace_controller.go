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
	"slices"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/labels"
	"k8s.io/apimachinery/pkg/types"
	rbacv1ac "k8s.io/client-go/applyconfigurations/rbac/v1"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/builder"
	"sigs.k8s.io/controller-runtime/pkg/cache"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/event"
	"sigs.k8s.io/controller-runtime/pkg/handler"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/predicate"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"
	"sigs.k8s.io/controller-runtime/pkg/source"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

var globalNSReconcileKey = reconcile.Request{NamespacedName: types.NamespacedName{Name: globalNSReconcileKeyStr}}

// GlobalNamespaceReconciler grants the platform Auth CR groups access to DCH
// resources in every labeled global namespace and records the global
// namespaces on every DataConnectService.
//
// In each global namespace:
//   - Auth allowedGroups and adminGroups can read connections and connection
//     types and ingest data (dch-read);
//   - Auth adminGroups can also create, modify, and delete them (dch-read-write);
//   - the REST and Flight ServiceAccounts can manage and read the connection
//     Secrets stored there.
//
// Kubernetes Secret access for users is deliberately not granted.
//
// All state is derived from the cluster, so every event maps to one
// reconcile key.
type GlobalNamespaceReconciler struct {
	client.Client
	Config GlobalNamespaceConfig

	// RoleBindings reads the RoleBindings this controller manages. Set up
	// by SetupWithManager as a cache limited to managed bindings.
	RoleBindings client.Reader
}

// +kubebuilder:rbac:groups="",resources=namespaces,verbs=get;list;watch
// +kubebuilder:rbac:groups=services.platform.opendatahub.io,resources=auths,verbs=get;list;watch
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=rolebindings,verbs=get;list;watch;create
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=rolebindings,resourceNames=dch-global-read;dch-global-read-write;dch-global-rest-service-secret-access;dch-global-flight-service-secret-access,verbs=update;patch;delete
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=clusterroles,resourceNames=dch-read;dch-read-write;dch-rest-service-secret-access;dch-flight-service-secret-access,verbs=bind

func (r *GlobalNamespaceReconciler) Reconcile(ctx context.Context, _ ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	var services dchv1alpha1.DataConnectServiceList
	if err := r.List(ctx, &services); err != nil {
		return ctrl.Result{}, fmt.Errorf("listing DataConnectService: %w", err)
	}
	active := slices.DeleteFunc(slices.Clone(services.Items), func(cr dchv1alpha1.DataConnectService) bool {
		return !cr.DeletionTimestamp.IsZero()
	})
	if len(active) == 0 {
		// Nothing to serve. RoleBindings cannot carry an owner reference to
		// the namespaced DataConnectService because they live in other
		// namespaces, so they are removed here instead of by the GC.
		return ctrl.Result{}, r.deleteManagedRoleBindings(ctx, nil)
	}

	labeled, err := labeledGlobalNamespaces(ctx, r, r.Config)
	if err != nil {
		return ctrl.Result{}, err
	}

	if len(labeled) > 0 {
		allowed, admin, err := authGroups(ctx, r)
		switch {
		case apierrors.IsNotFound(err) || meta.IsNoMatchError(err):
			// Without the Auth CR there are no groups to grant. The service
			// accounts keep their Secret access, which does not depend on it.
			log.Info("platform Auth CR not found, granting no group access in global namespaces")
			allowed, admin = nil, nil
		case err != nil:
			return ctrl.Result{}, fmt.Errorf("reading Auth %q: %w", platformAuthCRName, err)
		}
		restSAs, flightSAs := serviceAccountsFor(active)
		var errs []error
		for _, ns := range labeled {
			desired := desiredGlobalRoleBindings(ns, allowed, admin, restSAs, flightSAs)
			errs = append(errs, r.applyRoleBindings(ctx, desired))
		}
		if err := errors.Join(errs...); err != nil {
			return ctrl.Result{}, err
		}
	}

	// Remove bindings from namespaces that are no longer global.
	if err := r.deleteManagedRoleBindings(ctx, labeled); err != nil {
		return ctrl.Result{}, err
	}
	log.V(1).Info("reconciled global namespaces", "namespaces", labeled)
	return ctrl.Result{}, nil
}

// applyRoleBindings creates or updates the desired bindings.
func (r *GlobalNamespaceReconciler) applyRoleBindings(ctx context.Context, desired []*rbacv1.RoleBinding) error {
	var errs []error
	for _, rb := range desired {
		if err := r.applyRoleBinding(ctx, rb); err != nil {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

// applyRoleBinding server-side applies the desired binding, which creates it,
// adopts an existing binding with the same name, and repairs drift in one call.
func (r *GlobalNamespaceReconciler) applyRoleBinding(ctx context.Context, desired *rbacv1.RoleBinding) error {
	key := client.ObjectKeyFromObject(desired)
	err := r.Apply(ctx, roleBindingApplyConfig(desired), client.FieldOwner(globalNSFieldOwner), client.ForceOwnership)
	if isRoleRefChange(err) {
		// roleRef is immutable; a binding pointing elsewhere is recreated.
		logf.FromContext(ctx).Info("recreating global namespace RoleBinding with a different roleRef",
			"namespace", desired.Namespace, "name", desired.Name, "desiredRoleRef", desired.RoleRef.Name)
		stale := &rbacv1.RoleBinding{ObjectMeta: metav1.ObjectMeta{Namespace: desired.Namespace, Name: desired.Name}}
		if err := r.Delete(ctx, stale); err != nil && !apierrors.IsNotFound(err) {
			return fmt.Errorf("deleting RoleBinding %s: %w", key, err)
		}
		err = r.Apply(ctx, roleBindingApplyConfig(desired), client.FieldOwner(globalNSFieldOwner), client.ForceOwnership)
	}
	if err != nil {
		return fmt.Errorf("applying RoleBinding %s: %w", key, err)
	}
	return nil
}

func roleBindingApplyConfig(rb *rbacv1.RoleBinding) *rbacv1ac.RoleBindingApplyConfiguration {
	ac := rbacv1ac.RoleBinding(rb.Name, rb.Namespace).
		WithLabels(rb.Labels).
		WithRoleRef(rbacv1ac.RoleRef().
			WithAPIGroup(rb.RoleRef.APIGroup).
			WithKind(rb.RoleRef.Kind).
			WithName(rb.RoleRef.Name))
	for _, s := range rb.Subjects {
		sub := rbacv1ac.Subject().WithKind(s.Kind).WithName(s.Name)
		if s.APIGroup != "" {
			sub.WithAPIGroup(s.APIGroup)
		}
		if s.Namespace != "" {
			sub.WithNamespace(s.Namespace)
		}
		ac.WithSubjects(sub)
	}
	return ac
}

// isRoleRefChange reports whether the API server rejected an apply because it
// would change the immutable roleRef.
func isRoleRefChange(err error) bool {
	var status apierrors.APIStatus
	if !apierrors.IsInvalid(err) || !errors.As(err, &status) {
		return false
	}
	if d := status.Status().Details; d != nil {
		for _, c := range d.Causes {
			if c.Field == "roleRef" {
				return true
			}
		}
	}
	return false
}

func (r *GlobalNamespaceReconciler) deleteRoleBinding(ctx context.Context, namespace, name string) error {
	existing := &rbacv1.RoleBinding{}
	if err := r.RoleBindings.Get(ctx, types.NamespacedName{Namespace: namespace, Name: name}, existing); err != nil {
		if apierrors.IsNotFound(err) {
			return nil
		}
		return fmt.Errorf("getting RoleBinding %s/%s: %w", namespace, name, err)
	}
	logf.FromContext(ctx).Info("deleting global namespace RoleBinding", "namespace", namespace, "name", name)
	if err := r.Delete(ctx, existing); err != nil && !apierrors.IsNotFound(err) {
		return fmt.Errorf("deleting RoleBinding %s/%s: %w", namespace, name, err)
	}
	return nil
}

// deleteManagedRoleBindings removes managed bindings in every namespace not
// in keep; an empty keep removes them everywhere.
func (r *GlobalNamespaceReconciler) deleteManagedRoleBindings(ctx context.Context, keep []string) error {
	var list rbacv1.RoleBindingList
	if err := r.RoleBindings.List(ctx, &list, client.MatchingLabels{labelGlobalNamespace: valueTrue}); err != nil {
		return fmt.Errorf("listing global namespace RoleBindings: %w", err)
	}
	var errs []error
	for i := range list.Items {
		rb := &list.Items[i]
		if slices.Contains(keep, rb.Namespace) || !slices.Contains(managedGlobalRoleBindings, rb.Name) {
			continue
		}
		if err := r.deleteRoleBinding(ctx, rb.Namespace, rb.Name); err != nil {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

// SetupWithManager registers the controller. RoleBindings are watched through
// a dedicated cache limited to managed bindings so the manager never caches
// every RoleBinding in the cluster.
func (r *GlobalNamespaceReconciler) SetupWithManager(mgr ctrl.Manager) error {
	rbCache, err := cache.New(mgr.GetConfig(), cache.Options{
		Scheme: mgr.GetScheme(),
		Mapper: mgr.GetRESTMapper(),
		ByObject: map[client.Object]cache.ByObject{
			&rbacv1.RoleBinding{}: {Label: labels.SelectorFromSet(labels.Set{labelGlobalNamespace: valueTrue})},
		},
	})
	if err != nil {
		return fmt.Errorf("creating RoleBinding cache: %w", err)
	}
	if err := mgr.Add(rbCache); err != nil {
		return fmt.Errorf("adding RoleBinding cache: %w", err)
	}
	r.RoleBindings = rbCache

	auth := &unstructured.Unstructured{}
	auth.SetGroupVersionKind(platformAuthGVK)

	enqueue := handler.EnqueueRequestsFromMapFunc(func(context.Context, client.Object) []reconcile.Request {
		return []reconcile.Request{globalNSReconcileKey}
	})

	return ctrl.NewControllerManagedBy(mgr).
		Named("globalnamespace").
		WatchesRawSource(source.Kind(mgr.GetCache(), &corev1.Namespace{},
			handler.TypedEnqueueRequestsFromMapFunc(func(context.Context, *corev1.Namespace) []reconcile.Request {
				return []reconcile.Request{globalNSReconcileKey}
			}),
			labelKeyPredicate[*corev1.Namespace](r.Config.LabelKey),
		)).
		Watches(auth, enqueue, builder.WithPredicates(predicate.NewPredicateFuncs(func(obj client.Object) bool {
			return obj.GetName() == platformAuthCRName
		}))).
		WatchesRawSource(source.Kind(rbCache, &rbacv1.RoleBinding{},
			handler.TypedEnqueueRequestsFromMapFunc(func(context.Context, *rbacv1.RoleBinding) []reconcile.Request {
				return []reconcile.Request{globalNSReconcileKey}
			}),
		)).
		Watches(&dchv1alpha1.DataConnectService{}, enqueue, builder.WithPredicates(dataConnectServicePredicate())).
		Complete(r)
}

// labelKeyPredicate passes events for objects that carry the label key, and
// updates where it was added, removed, or changed.
func labelKeyPredicate[T client.Object](key string) predicate.TypedPredicate[T] {
	has := func(obj T) bool {
		_, ok := obj.GetLabels()[key]
		return ok
	}
	return predicate.TypedFuncs[T]{
		CreateFunc: func(e event.TypedCreateEvent[T]) bool { return has(e.Object) },
		DeleteFunc: func(e event.TypedDeleteEvent[T]) bool { return has(e.Object) },
		UpdateFunc: func(e event.TypedUpdateEvent[T]) bool {
			if has(e.ObjectOld) != has(e.ObjectNew) {
				return true
			}
			if !has(e.ObjectNew) {
				return false
			}
			// A terminating global namespace must stop being global.
			return e.ObjectOld.GetLabels()[key] != e.ObjectNew.GetLabels()[key] ||
				e.ObjectOld.GetDeletionTimestamp().IsZero() != e.ObjectNew.GetDeletionTimestamp().IsZero()
		},
		GenericFunc: func(e event.TypedGenericEvent[T]) bool { return has(e.Object) },
	}
}

// dataConnectServicePredicate reacts to creation, deletion, and the start of
// deletion; those change the ServiceAccounts that need Secret access. Status
// updates written by this controller are ignored.
func dataConnectServicePredicate() predicate.Predicate {
	return predicate.Funcs{
		UpdateFunc: func(e event.UpdateEvent) bool {
			return e.ObjectOld.GetDeletionTimestamp().IsZero() != e.ObjectNew.GetDeletionTimestamp().IsZero()
		},
	}
}
