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
	"reflect"
	"slices"
	"time"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	logf "sigs.k8s.io/controller-runtime/pkg/log"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	idctFinalizerName    = "dataconnecthub.opendatahub.io/connection-type-finalizer"
	annotationResourceID = "dataconnecthub.opendatahub.io/resource-id"
	conditionTypeSynced  = "Synced"

	requeueOnServiceUnavailable = 15 * time.Second
)

// InitDataConnectionTypeReconciler reconciles InitDataConnectionType objects.
type InitDataConnectionTypeReconciler struct {
	client.Client
	Scheme     *runtime.Scheme
	RestClient ConnectionTypeClient
}

// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=initdataconnectiontypes,verbs=get;list;watch;update;patch
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=initdataconnectiontypes/status,verbs=get;update;patch
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=initdataconnectiontypes/finalizers,verbs=update

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

	// Handle deletion — clean up the REST resource then remove finalizer
	if !cr.DeletionTimestamp.IsZero() {
		if controllerutil.ContainsFinalizer(&cr, idctFinalizerName) {
			if err := r.handleDeletion(ctx, &cr); err != nil {
				if errors.Is(err, ErrServiceUnavailable) {
					log.Info("REST service unavailable during deletion, requeueing", "name", req.Name)
					return ctrl.Result{RequeueAfter: requeueOnServiceUnavailable}, nil
				}
				log.Error(err, "failed to delete connection type from REST", "name", req.Name)
				return ctrl.Result{RequeueAfter: requeueOnError}, nil
			}
			controllerutil.RemoveFinalizer(&cr, idctFinalizerName)
			return ctrl.Result{}, r.Update(ctx, &cr)
		}
		return ctrl.Result{}, nil
	}

	// Ensure finalizer is present
	if !controllerutil.ContainsFinalizer(&cr, idctFinalizerName) {
		controllerutil.AddFinalizer(&cr, idctFinalizerName)
		if err := r.Update(ctx, &cr); err != nil {
			return ctrl.Result{}, err
		}
	}

	// Sync the connection type to the REST service
	result, err := r.syncToREST(ctx, &cr)
	if err != nil {
		if errors.Is(err, ErrServiceUnavailable) {
			log.Info("REST service unavailable, requeueing", "name", req.Name)
			r.setConditionAndStatus(ctx, &cr, "Pending", metav1.ConditionFalse, "ServiceUnavailable", "REST service is not reachable")
			return ctrl.Result{RequeueAfter: requeueOnServiceUnavailable}, nil
		}
		log.Error(err, "failed to sync connection type", "name", req.Name)
		r.setConditionAndStatus(ctx, &cr, "Error", metav1.ConditionFalse, "SyncFailed", err.Error())
		return ctrl.Result{RequeueAfter: requeueOnError}, nil
	}

	if result == syncCreated || result == syncUpdated {
		log.Info("InitDataConnectionType synced", "name", req.Name, "action", result)
	}

	r.setConditionAndStatus(ctx, &cr, "Synced", metav1.ConditionTrue, "Synced", "Connection type is synced to the REST service")
	return ctrl.Result{}, nil
}

type syncResult string

const (
	syncCreated   syncResult = "created"
	syncUpdated   syncResult = "updated"
	syncUnchanged syncResult = "unchanged"
)

func (r *InitDataConnectionTypeReconciler) syncToREST(ctx context.Context, cr *dchv1alpha1.InitDataConnectionType) (syncResult, error) {
	desired := specToConnectionType(&cr.Spec)
	resourceID := cr.Annotations[annotationResourceID]

	if resourceID != "" {
		existing, err := r.RestClient.GetConnectionType(ctx, resourceID)
		if err != nil {
			if !errors.Is(err, ErrNotFound) {
				return "", err
			}
			// Resource was deleted externally — re-create
		} else {
			// Resource exists — check for delta
			if connectionTypesEqual(desired, existing.Resource) {
				return syncUnchanged, nil
			}
			if _, err := r.RestClient.UpdateConnectionType(ctx, resourceID, desired); err != nil {
				return "", err
			}
			return syncUpdated, nil
		}
	}

	// Create new resource
	resource, err := r.RestClient.CreateConnectionType(ctx, desired)
	if err != nil {
		return "", err
	}

	// Store the resource ID in an annotation
	if cr.Annotations == nil {
		cr.Annotations = make(map[string]string)
	}
	cr.Annotations[annotationResourceID] = resource.Metadata.ID
	if err := r.Update(ctx, cr); err != nil {
		return "", err
	}

	return syncCreated, nil
}

func (r *InitDataConnectionTypeReconciler) handleDeletion(ctx context.Context, cr *dchv1alpha1.InitDataConnectionType) error {
	resourceID := cr.Annotations[annotationResourceID]
	if resourceID == "" {
		return nil
	}

	err := r.RestClient.DeleteConnectionType(ctx, resourceID)
	if err != nil && !errors.Is(err, ErrNotFound) {
		return err
	}
	return nil
}

func (r *InitDataConnectionTypeReconciler) setConditionAndStatus(ctx context.Context, cr *dchv1alpha1.InitDataConnectionType, phase string, status metav1.ConditionStatus, reason, message string) {
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

// connectionTypesEqual compares two ConnectionType structs, normalizing
// nil vs empty slices to avoid false deltas from JSON deserialization.
func connectionTypesEqual(a, b ConnectionType) bool {
	normalizeFields := func(fields []Field) []Field {
		result := slices.Clone(fields)
		for i := range result {
			if result[i].EnumValues == nil {
				result[i].EnumValues = []EnumValue{}
			}
		}
		return result
	}
	a.CredentialsFields = normalizeFields(a.CredentialsFields)
	b.CredentialsFields = normalizeFields(b.CredentialsFields)
	return reflect.DeepEqual(a, b)
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
