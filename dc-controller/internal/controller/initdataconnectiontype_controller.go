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
	"bytes"
	"context"
	"encoding/json"
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
// It converges the connection type in the REST service to the CR's desired
// state: it creates the type when absent, patches it when the stored
// definition differs, and leaves it alone once synced for the current
// generation. A pre-existing type with the same name in the tenant —
// whether promoted from a ConfigMap or created directly — is adopted and
// overwritten to match the spec: the CR's name is authoritative, so custom
// types must use different names. Deleting the CR has no effect on the REST
// service — the connection type lives on independently.
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

	// Already converged for this generation — nothing left to do. The
	// historical "AlreadyExists" phase is excluded so CRs recorded by older
	// versions (where the stored type was never aligned with the spec) get
	// one converging pass after upgrade.
	if cr.Status.Phase == "Completed" && alreadySyncedForGeneration(&cr) {
		return ctrl.Result{}, nil
	}

	log.Info("syncing connection type", "name", cr.Spec.Name, "provider", cr.Spec.Provider)

	desired := specToConnectionType(&cr.Spec)

	items, err := r.RestClient.ListConnectionTypes(ctx, cr.Namespace)
	if err != nil {
		return r.handleSyncError(ctx, &cr, err, "listing connection types")
	}

	var existing *ConnectionTypeResource
	for i := range items {
		if items[i].Resource.Name == desired.Name {
			existing = &items[i]
			break
		}
	}

	switch {
	case existing == nil:
		if err := r.RestClient.CreateConnectionType(ctx, cr.Namespace, desired); err != nil {
			return r.handleSyncError(ctx, &cr, err, "registering connection type")
		}
		log.Info("connection type registered", "name", cr.Spec.Name)
		r.setStatus(ctx, &cr, "Completed", metav1.ConditionTrue, "Created", "Connection type registered in the REST service")

	case connectionTypesEqual(existing.Resource, desired):
		log.Info("connection type already up to date", "name", cr.Spec.Name)
		r.setStatus(ctx, &cr, "Completed", metav1.ConditionTrue, "UpToDate", "Connection type already matches the desired state")

	default:
		if err := r.RestClient.UpdateConnectionType(ctx, cr.Namespace, existing.Metadata.ID, desired); err != nil {
			return r.handleSyncError(ctx, &cr, err, "updating connection type")
		}
		log.Info("connection type updated", "name", cr.Spec.Name)
		r.setStatus(ctx, &cr, "Completed", metav1.ConditionTrue, "Updated", "Connection type updated in the REST service")
	}

	return ctrl.Result{}, nil
}

// handleSyncError maps a REST failure to a status update. Unavailability and
// races against concurrent writers (create conflict, update not-found) are
// transient: the CR is requeued so the next reconcile converges. Any other
// failure is recorded as a terminal Error until the spec changes again.
func (r *InitDataConnectionTypeReconciler) handleSyncError(ctx context.Context, cr *dchv1alpha1.InitDataConnectionType, err error, action string) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	switch {
	case errors.Is(err, ErrServiceUnavailable):
		log.Info("REST service unavailable, requeuing", "name", cr.Spec.Name)
		r.setStatus(ctx, cr, "Pending", metav1.ConditionFalse, "ServiceUnavailable", "REST service is not reachable")
		return ctrl.Result{RequeueAfter: requeueOnServiceUnavailable}, nil
	case errors.Is(err, ErrConflict):
		// Created concurrently since the list — requeue; the next reconcile
		// finds it and patches.
		log.Info("connection type created concurrently, requeuing", "name", cr.Spec.Name)
		r.setStatus(ctx, cr, "Pending", metav1.ConditionFalse, "Conflict", "Connection type created concurrently, retrying")
		return ctrl.Result{RequeueAfter: requeueOnServiceUnavailable}, nil
	case errors.Is(err, ErrNotFound):
		// Deleted concurrently since the list — requeue to re-create.
		log.Info("connection type deleted concurrently, requeuing", "name", cr.Spec.Name)
		r.setStatus(ctx, cr, "Pending", metav1.ConditionFalse, "NotFound", "Connection type deleted concurrently, retrying")
		return ctrl.Result{RequeueAfter: requeueOnServiceUnavailable}, nil
	}

	log.Error(err, "failed to sync connection type", "name", cr.Spec.Name, "action", action)
	r.setStatus(ctx, cr, "Error", metav1.ConditionFalse, "SyncFailed", err.Error())
	return ctrl.Result{}, nil
}

// alreadySyncedForGeneration reports whether the Synced condition is True and
// was recorded for the CR's current generation.
func alreadySyncedForGeneration(cr *dchv1alpha1.InitDataConnectionType) bool {
	cond := meta.FindStatusCondition(cr.Status.Conditions, conditionTypeSynced)
	return cond != nil &&
		cond.Status == metav1.ConditionTrue &&
		cond.ObservedGeneration == cr.Generation
}

// connectionTypesEqual compares two connection types by their canonical JSON
// form so nil and empty optional fields compare consistently.
func connectionTypesEqual(a, b ConnectionType) bool {
	aj, err := json.Marshal(a)
	if err != nil {
		return false
	}
	bj, err := json.Marshal(b)
	if err != nil {
		return false
	}
	return bytes.Equal(aj, bj)
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
		Label:             spec.Label,
		Provider:          spec.Provider,
		Description:       spec.Description,
		CredentialsFields: fields,
		Tags:              spec.Tags,
	}
}

// SetupWithManager sets up the controller with the Manager.
func (r *InitDataConnectionTypeReconciler) SetupWithManager(mgr ctrl.Manager) error {
	return ctrl.NewControllerManagedBy(mgr).
		For(&dchv1alpha1.InitDataConnectionType{}).
		Named("initdataconnectiontype").
		Complete(r)
}
