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
	"time"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
	logf "sigs.k8s.io/controller-runtime/pkg/log"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	conditionTypeSynced = "Synced"

	requeueOnServiceUnavailable = 15 * time.Second
)

// InitDataConnectionTypeReconciler reconciles InitDataConnectionType objects.
// It is a one-shot controller: on first reconcile it POSTs the connection type
// to the REST service. Once completed (or if the type already exists), it sets
// a terminal status and does not reconcile again. Deleting the CR has no effect
// on the REST service — the connection type lives on independently.
type InitDataConnectionTypeReconciler struct {
	client.Client
	Scheme     *runtime.Scheme
	RestClient ConnectionTypeClient
}

// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=initdataconnectiontypes,verbs=get;list;watch
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=initdataconnectiontypes/status,verbs=get;update;patch

func (r *InitDataConnectionTypeReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	var cr dchv1alpha1.InitDataConnectionType
	if err := r.Get(ctx, req.NamespacedName, &cr); err != nil {
		if apierrors.IsNotFound(err) {
			return ctrl.Result{}, nil
		}
		return ctrl.Result{}, err
	}

	// Terminal states — nothing left to do
	if cr.Status.Phase == "Completed" || cr.Status.Phase == "AlreadyExists" {
		return ctrl.Result{}, nil
	}

	log.Info("registering connection type", "name", cr.Spec.Name, "provider", cr.Spec.Provider)

	desired := specToConnectionType(&cr.Spec)
	err := r.RestClient.CreateConnectionType(ctx, cr.Namespace, desired)

	if err == nil {
		log.Info("connection type registered", "name", cr.Spec.Name)
		r.setStatus(ctx, &cr, "Completed", metav1.ConditionTrue, "Created", "Connection type registered in the REST service")
		return ctrl.Result{}, nil
	}

	if errors.Is(err, ErrConflict) {
		log.Info("connection type already exists", "name", cr.Spec.Name)
		r.setStatus(ctx, &cr, "AlreadyExists", metav1.ConditionTrue, "AlreadyExists", "Connection type already exists in the REST service")
		return ctrl.Result{}, nil
	}

	if errors.Is(err, ErrServiceUnavailable) {
		log.Info("REST service unavailable, requeuing", "name", cr.Spec.Name)
		r.setStatus(ctx, &cr, "Pending", metav1.ConditionFalse, "ServiceUnavailable", "REST service is not reachable")
		return ctrl.Result{RequeueAfter: requeueOnServiceUnavailable}, nil
	}

	log.Error(err, "failed to register connection type", "name", cr.Spec.Name)
	r.setStatus(ctx, &cr, "Error", metav1.ConditionFalse, "SyncFailed", err.Error())
	return ctrl.Result{}, nil
}

func (r *InitDataConnectionTypeReconciler) setStatus(ctx context.Context, cr *dchv1alpha1.InitDataConnectionType, phase string, status metav1.ConditionStatus, reason, message string) {
	cr.Status.Phase = phase
	meta.SetStatusCondition(&cr.Status.Conditions, metav1.Condition{
		Type:               conditionTypeSynced,
		Status:             status,
		ObservedGeneration: cr.Generation,
		Reason:             reason,
		Message:            message,
	})
	if err := r.Status().Update(ctx, cr); err != nil {
		logf.FromContext(ctx).Error(err, "failed to update status", "name", cr.Name)
	}
}

// specToConnectionType maps a CRD spec to the REST API payload.
func specToConnectionType(spec *dchv1alpha1.InitDataConnectionTypeSpec) ConnectionType {
	fields := make([]Field, len(spec.CredentialsFields))
	for i, cf := range spec.CredentialsFields {
		var enumValues []EnumValue
		for _, ev := range cf.EnumValues {
			enumValues = append(enumValues, EnumValue{
				Value: ev.Value,
				Label: ev.Label,
			})
		}
		fields[i] = Field{
			Name:         cf.Name,
			Label:        cf.Label,
			Description:  cf.Description,
			Required:     cf.Required,
			Type:         cf.Type,
			EnumValues:   enumValues,
			DefaultValue: cf.DefaultValue,
		}
	}

	return ConnectionType{
		Name:              spec.Name,
		Provider:          spec.Provider,
		Description:       spec.Description,
		CredentialsFields: fields,
	}
}

// SetupWithManager sets up the controller with the Manager.
func (r *InitDataConnectionTypeReconciler) SetupWithManager(mgr ctrl.Manager) error {
	return ctrl.NewControllerManagedBy(mgr).
		For(&dchv1alpha1.InitDataConnectionType{}).
		Named("initdataconnectiontype").
		Complete(r)
}
