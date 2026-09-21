package tls

import (
	"context"
	"reflect"

	configv1 "github.com/openshift/api/config/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"sigs.k8s.io/controller-runtime/pkg/builder"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller"
	"sigs.k8s.io/controller-runtime/pkg/event"
	"sigs.k8s.io/controller-runtime/pkg/predicate"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	ctrl "sigs.k8s.io/controller-runtime"
)

type ProfileWatcher struct {
	client.Client
	InitialProfile          *configv1.TLSSecurityProfile
	InitialAdherencePolicy  configv1.TLSAdherencePolicy
	OnProfileChange         func(context.Context)
	OnAdherencePolicyChange func(context.Context)

	lastProfile         *configv1.TLSSecurityProfile
	lastAdherencePolicy configv1.TLSAdherencePolicy
}

func (w *ProfileWatcher) Reconcile(ctx context.Context, req reconcile.Request) (reconcile.Result, error) {
	if req.Name != apiServerName {
		return reconcile.Result{}, nil
	}

	apiServer := &configv1.APIServer{}
	if err := w.Get(ctx, req.NamespacedName, apiServer); err != nil {
		if apierrors.IsNotFound(err) {
			if w.OnProfileChange != nil {
				w.OnProfileChange(ctx)
			}
			return reconcile.Result{}, nil
		}
		return reconcile.Result{}, client.IgnoreNotFound(err)
	}

	currentProfile := cloneProfile(apiServer.Spec.TLSSecurityProfile)
	if !reflect.DeepEqual(w.lastProfile, currentProfile) {
		w.lastProfile = currentProfile
		if w.OnProfileChange != nil {
			w.OnProfileChange(ctx)
		}
	}

	currentAdherencePolicy := apiServer.Spec.TLSAdherence
	if w.lastAdherencePolicy != currentAdherencePolicy {
		w.lastAdherencePolicy = currentAdherencePolicy
		if w.OnAdherencePolicyChange != nil {
			w.OnAdherencePolicyChange(ctx)
		}
	}
	return reconcile.Result{}, nil
}

func (w *ProfileWatcher) SetupWithManager(mgr ctrl.Manager) error {
	w.lastProfile = cloneProfile(w.InitialProfile)
	w.lastAdherencePolicy = w.InitialAdherencePolicy
	return ctrl.NewControllerManagedBy(mgr).
		Named("tls-profile-watcher").
		WithOptions(controller.Options{NeedLeaderElection: ptr(false)}).
		For(&configv1.APIServer{}, builder.WithPredicates(predicate.Funcs{
			CreateFunc:  func(e event.CreateEvent) bool { return e.Object.GetName() == apiServerName },
			UpdateFunc:  func(e event.UpdateEvent) bool { return e.ObjectNew.GetName() == apiServerName },
			DeleteFunc:  func(e event.DeleteEvent) bool { return e.Object.GetName() == apiServerName },
			GenericFunc: func(e event.GenericEvent) bool { return e.Object.GetName() == apiServerName },
		})).
		Complete(w)
}

func ptr(value bool) *bool { return &value }
