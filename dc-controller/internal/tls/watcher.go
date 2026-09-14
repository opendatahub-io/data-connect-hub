package tls

import (
	"context"
	"reflect"

	configv1 "github.com/openshift/api/config/v1"
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
	InitialProfileSpec configv1.TLSProfileSpec
	OnProfileChange    func(context.Context)

	lastProfile configv1.TLSProfileSpec
}

func (w *ProfileWatcher) Reconcile(ctx context.Context, req reconcile.Request) (reconcile.Result, error) {
	if req.Name != "cluster" {
		return reconcile.Result{}, nil
	}

	apiServer := &configv1.APIServer{}
	if err := w.Get(ctx, req.NamespacedName, apiServer); err != nil {
		return reconcile.Result{}, client.IgnoreNotFound(err)
	}

	current := profileSpec(apiServer.Spec.TLSSecurityProfile)
	if w.OnProfileChange != nil && !reflect.DeepEqual(w.lastProfile, current) {
		w.lastProfile = current
		w.OnProfileChange(ctx)
	}
	return reconcile.Result{}, nil
}

func (w *ProfileWatcher) SetupWithManager(mgr ctrl.Manager) error {
	w.lastProfile = w.InitialProfileSpec
	return ctrl.NewControllerManagedBy(mgr).
		Named("tls-profile-watcher").
		WithOptions(controller.Options{NeedLeaderElection: ptr(false)}).
		For(&configv1.APIServer{}, builder.WithPredicates(predicate.Funcs{
			CreateFunc:  func(e event.CreateEvent) bool { return e.Object.GetName() == "cluster" },
			UpdateFunc:  func(e event.UpdateEvent) bool { return e.ObjectNew.GetName() == "cluster" },
			DeleteFunc:  func(e event.DeleteEvent) bool { return e.Object.GetName() == "cluster" },
			GenericFunc: func(e event.GenericEvent) bool { return e.Object.GetName() == "cluster" },
		})).
		Complete(w)
}

func ptr(value bool) *bool { return &value }
