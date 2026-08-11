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

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/runtime"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
	logf "sigs.k8s.io/controller-runtime/pkg/log"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// InitDataConnectionTypeReconciler reconciles InitDataConnectionType objects.
type InitDataConnectionTypeReconciler struct {
	client.Client
	Scheme *runtime.Scheme
}

// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=initdataconnectiontypes,verbs=get;list;watch
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=initdataconnectiontypes/status,verbs=get;update;patch

func (r *InitDataConnectionTypeReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	var cr dchv1alpha1.InitDataConnectionType
	if err := r.Get(ctx, req.NamespacedName, &cr); err != nil {
		if apierrors.IsNotFound(err) {
			log.Info("InitDataConnectionType deleted", "name", req.Name)
			return ctrl.Result{}, nil
		}
		return ctrl.Result{}, err
	}

	log.Info("InitDataConnectionType reconciled",
		"name", req.Name,
		"provider", cr.Spec.Provider,
		"connectionTypeName", cr.Spec.Name,
		"generation", cr.Generation,
	)

	return ctrl.Result{}, nil
}

// SetupWithManager sets up the controller with the Manager.
func (r *InitDataConnectionTypeReconciler) SetupWithManager(mgr ctrl.Manager) error {
	return ctrl.NewControllerManagedBy(mgr).
		For(&dchv1alpha1.InitDataConnectionType{}).
		Named("initdataconnectiontype").
		Complete(r)
}
