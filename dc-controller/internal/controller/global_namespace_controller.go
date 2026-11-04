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
	"maps"
	"slices"
	"strings"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	"k8s.io/apimachinery/pkg/api/equality"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/labels"
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
	"sigs.k8s.io/controller-runtime/pkg/source"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

var globalNamespaceReconcileKey = reconcile.Request{NamespacedName: types.NamespacedName{Name: globalNamespaceReconcileKeyStr}}

// GlobalNamespaceReconciler grants the platform Auth CR groups access to DCH
// resources in the global namespace and records the effective global
// namespace on every DataConnectService.
//
// In the global namespace:
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
	// APIReader reads RoleBindings that the label-filtered cache cannot see,
	// e.g. an unlabeled binding that already uses a managed name.
	APIReader client.Reader
	Config    GlobalNamespaceConfig

	// RoleBindings reads the RoleBindings this controller manages. Set up
	// by SetupWithManager as a cache limited to managed bindings.
	RoleBindings client.Reader
}

// +kubebuilder:rbac:groups="",resources=namespaces,verbs=get;list;watch
// +kubebuilder:rbac:groups=services.platform.opendatahub.io,resources=auths,verbs=get;list;watch
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=rolebindings,verbs=get;list;watch;create
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=rolebindings,resourceNames=dch-global-read;dch-global-read-write;dch-global-rest-secret-access;dch-global-flight-secret-read,verbs=update;patch;delete
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=clusterroles,resourceNames=dch-read;dch-read-write;dch-rest-service-secret-access;dch-flight-secret-read,verbs=bind

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
		// Nothing to serve; the DataConnectService finalizer also removes
		// the bindings, this covers bindings left behind by a crash.
		return ctrl.Result{}, r.deleteManagedRoleBindings(ctx, "")
	}

	res, err := resolveGlobalNamespace(ctx, r, r.Config, active[0].Status.GlobalNamespace)
	if err != nil {
		return ctrl.Result{}, err
	}

	cond := metav1.Condition{Type: conditionTypeGlobalNSReady}
	var reconcileErr error
	switch {
	case res.Effective == "" && len(res.Labeled) == 0:
		cond.Status, cond.Reason = metav1.ConditionTrue, reasonGlobalNSNotConfigured
		cond.Message = fmt.Sprintf("No namespace is labeled %s", r.Config.LabelKey)
	case res.Ambiguous():
		cond.Status, cond.Reason = metav1.ConditionFalse, reasonGlobalNSMultiple
		cond.Message = fmt.Sprintf("Only one global namespace is supported, found %d labeled %s: %s; keeping %q",
			len(res.Labeled), r.Config.LabelKey, strings.Join(res.Labeled, ", "), res.Effective)
	default:
		cond.Status, cond.Reason = metav1.ConditionTrue, reasonGlobalNSReconciled
		cond.Message = fmt.Sprintf("RoleBindings reconciled in namespace %s", res.Effective)
	}

	if res.Effective != "" {
		allowed, admin, err := authGroups(ctx, r)
		switch {
		case apierrors.IsNotFound(err) || meta.IsNoMatchError(err):
			// Without the Auth CR there are no groups to grant, so any
			// previously granted access must not outlive it.
			log.Info("platform Auth CR not found, removing global namespace RoleBindings")
			cond.Status, cond.Reason = metav1.ConditionFalse, reasonGlobalNSAuthNotFound
			cond.Message = fmt.Sprintf("Auth %q not found; no access is granted in namespace %s", authCRName, res.Effective)
			reconcileErr = r.deleteManagedRoleBindings(ctx, "")
		case err != nil:
			reconcileErr = fmt.Errorf("reading Auth %q: %w", authCRName, err)
		default:
			restSAs, flightSAs := serviceAccountsFor(active)
			desired := desiredGlobalRoleBindings(res.Effective, allowed, admin, restSAs, flightSAs)
			reconcileErr = r.applyRoleBindings(ctx, res.Effective, desired)
		}
	}

	// Remove bindings from namespaces that are no longer global.
	if reconcileErr == nil {
		reconcileErr = r.deleteManagedRoleBindings(ctx, res.Effective)
	}
	if reconcileErr != nil {
		cond.Status, cond.Reason = metav1.ConditionFalse, reasonGlobalNSReconcileFailed
		cond.Message = reconcileErr.Error()
	}

	statusErr := r.updateStatus(ctx, active, res.Effective, cond)
	if err := errors.Join(reconcileErr, statusErr); err != nil {
		return ctrl.Result{}, err
	}
	log.V(1).Info("reconciled global namespace", "namespace", res.Effective, "labeled", res.Labeled)
	return ctrl.Result{}, nil
}

// applyRoleBindings creates or updates the desired bindings and deletes the
// managed names that are no longer desired, e.g. when a group list empties.
func (r *GlobalNamespaceReconciler) applyRoleBindings(ctx context.Context, namespace string, desired []*rbacv1.RoleBinding) error {
	var errs []error
	wanted := make(map[string]bool, len(desired))
	for _, rb := range desired {
		wanted[rb.Name] = true
		if err := r.applyRoleBinding(ctx, rb); err != nil {
			errs = append(errs, err)
		}
	}
	for _, name := range managedGlobalRoleBindings {
		if wanted[name] {
			continue
		}
		if err := r.deleteRoleBinding(ctx, namespace, name); err != nil {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

func (r *GlobalNamespaceReconciler) applyRoleBinding(ctx context.Context, desired *rbacv1.RoleBinding) error {
	log := logf.FromContext(ctx)
	key := client.ObjectKeyFromObject(desired)

	existing := &rbacv1.RoleBinding{}
	err := r.RoleBindings.Get(ctx, key, existing)
	if apierrors.IsNotFound(err) {
		// The cache only holds labeled bindings; an unlabeled one with a
		// managed name is adopted.
		err = r.APIReader.Get(ctx, key, existing)
	}
	if apierrors.IsNotFound(err) {
		log.Info("creating global namespace RoleBinding", "namespace", desired.Namespace, "name", desired.Name)
		if err := r.Create(ctx, desired.DeepCopy()); err != nil {
			return fmt.Errorf("creating RoleBinding %s: %w", key, err)
		}
		return nil
	}
	if err != nil {
		return fmt.Errorf("getting RoleBinding %s: %w", key, err)
	}

	// roleRef is immutable; a binding pointing elsewhere is recreated.
	if existing.RoleRef != desired.RoleRef {
		log.Info("recreating global namespace RoleBinding with a different roleRef",
			"namespace", desired.Namespace, "name", desired.Name,
			"existingRoleRef", existing.RoleRef.Name, "desiredRoleRef", desired.RoleRef.Name)
		if err := r.Delete(ctx, existing); err != nil && !apierrors.IsNotFound(err) {
			return fmt.Errorf("deleting RoleBinding %s: %w", key, err)
		}
		if err := r.Create(ctx, desired.DeepCopy()); err != nil {
			return fmt.Errorf("creating RoleBinding %s: %w", key, err)
		}
		return nil
	}

	labelsMatch := true
	for k, v := range desired.Labels {
		if existing.Labels[k] != v {
			labelsMatch = false
			break
		}
	}
	if labelsMatch && equality.Semantic.DeepEqual(existing.Subjects, desired.Subjects) {
		return nil
	}

	updated := existing.DeepCopy()
	if updated.Labels == nil {
		updated.Labels = map[string]string{}
	}
	maps.Copy(updated.Labels, desired.Labels)
	updated.Subjects = desired.Subjects
	log.Info("updating global namespace RoleBinding", "namespace", desired.Namespace, "name", desired.Name)
	if err := r.Update(ctx, updated); err != nil {
		return fmt.Errorf("updating RoleBinding %s: %w", key, err)
	}
	return nil
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

// deleteManagedRoleBindings removes managed bindings in every namespace
// except keep; an empty keep removes them everywhere.
func (r *GlobalNamespaceReconciler) deleteManagedRoleBindings(ctx context.Context, keep string) error {
	var list rbacv1.RoleBindingList
	if err := r.RoleBindings.List(ctx, &list, client.MatchingLabels{labelGlobalNamespace: valueTrue}); err != nil {
		return fmt.Errorf("listing global namespace RoleBindings: %w", err)
	}
	var errs []error
	for i := range list.Items {
		rb := &list.Items[i]
		if rb.Namespace == keep || !slices.Contains(managedGlobalRoleBindings, rb.Name) {
			continue
		}
		if err := r.deleteRoleBinding(ctx, rb.Namespace, rb.Name); err != nil {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

func (r *GlobalNamespaceReconciler) updateStatus(ctx context.Context, items []dchv1alpha1.DataConnectService, effective string, cond metav1.Condition) error {
	var errs []error
	for i := range items {
		key := client.ObjectKeyFromObject(&items[i])
		var cr dchv1alpha1.DataConnectService
		if err := r.Get(ctx, key, &cr); err != nil {
			if apierrors.IsNotFound(err) {
				continue
			}
			errs = append(errs, err)
			continue
		}
		c := cond
		c.ObservedGeneration = cr.Generation
		changed := cr.Status.GlobalNamespace != effective
		cr.Status.GlobalNamespace = effective
		if meta.SetStatusCondition(&cr.Status.Conditions, c) {
			changed = true
		}
		if !changed {
			continue
		}
		if err := r.Status().Update(ctx, &cr); err != nil {
			errs = append(errs, fmt.Errorf("updating DataConnectService %s status: %w", key, err))
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
	if r.APIReader == nil {
		r.APIReader = mgr.GetAPIReader()
	}

	auth := &unstructured.Unstructured{}
	auth.SetGroupVersionKind(authGVK)

	enqueue := handler.EnqueueRequestsFromMapFunc(func(context.Context, client.Object) []reconcile.Request {
		return []reconcile.Request{globalNamespaceReconcileKey}
	})

	return ctrl.NewControllerManagedBy(mgr).
		Named("globalnamespace").
		WatchesRawSource(source.Kind(mgr.GetCache(), &corev1.Namespace{},
			handler.TypedEnqueueRequestsFromMapFunc(func(context.Context, *corev1.Namespace) []reconcile.Request {
				return []reconcile.Request{globalNamespaceReconcileKey}
			}),
			labelKeyPredicate[*corev1.Namespace](r.Config.LabelKey),
		)).
		Watches(auth, enqueue, builder.WithPredicates(predicate.NewPredicateFuncs(func(obj client.Object) bool {
			return obj.GetName() == authCRName
		}))).
		WatchesRawSource(source.Kind(rbCache, &rbacv1.RoleBinding{},
			handler.TypedEnqueueRequestsFromMapFunc(func(context.Context, *rbacv1.RoleBinding) []reconcile.Request {
				return []reconcile.Request{globalNamespaceReconcileKey}
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
