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
	"fmt"
	"reflect"
	"strings"
	"time"

	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/builder"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	"sigs.k8s.io/controller-runtime/pkg/handler"
	"sigs.k8s.io/controller-runtime/pkg/predicate"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
	componentsplatformv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/v1alpha1"
)

const (
	dataConnectHubName             = "default-dataconnecthub"
	dataConnectHubKind             = "DataConnectHub"
	dataConnectServiceName         = "default-dataconnectservice"
	dataConnectHubFinalizer        = "components.platform.opendatahub.io/finalizer"
	modulePlatformConfigName       = "odh-dataconnecthub-config"
	platformVersionKey             = "platformVersion"
	reconcilePendingInterval       = 10 * time.Second
	conditionReady                 = "Ready"
	conditionProvisioningSucceeded = "ProvisioningSucceeded"
	conditionDegraded              = "Degraded"
)

// DataConnectHubReconciler bridges the cluster-scoped platform module API to
// the existing namespaced DataConnectService application API.
type DataConnectHubReconciler struct {
	client.Client
	Scheme *runtime.Scheme
}

// +kubebuilder:rbac:groups=components.platform.opendatahub.io,resources=dataconnecthubs,verbs=get;list;watch;update;patch
// +kubebuilder:rbac:groups=components.platform.opendatahub.io,resources=dataconnecthubs/status,verbs=get;update;patch
// +kubebuilder:rbac:groups=components.platform.opendatahub.io,resources=dataconnecthubs/finalizers,verbs=update
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices/status,verbs=get;list;watch
// +kubebuilder:rbac:groups="",resources=configmaps,verbs=get;list;watch;create;update;patch

func (r *DataConnectHubReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := ctrl.LoggerFrom(ctx)
	hub := &componentsplatformv1alpha1.DataConnectHub{}
	if err := r.Get(ctx, types.NamespacedName{Name: req.Name}, hub); err != nil {
		if apierrors.IsNotFound(err) {
			return ctrl.Result{}, nil
		}
		return ctrl.Result{}, fmt.Errorf("getting DataConnectHub: %w", err)
	}

	if !hub.DeletionTimestamp.IsZero() {
		return r.reconcileDelete(ctx, hub)
	}
	if !controllerutil.ContainsFinalizer(hub, dataConnectHubFinalizer) {
		controllerutil.AddFinalizer(hub, dataConnectHubFinalizer)
		if err := r.Update(ctx, hub); err != nil {
			return ctrl.Result{}, fmt.Errorf("adding DataConnectHub finalizer: %w", err)
		}
		return ctrl.Result{Requeue: true}, nil
	}
	if hub.Name != dataConnectHubName {
		return r.updateFailure(ctx, hub, "InvalidName", "DataConnectHub must use the name default-dataconnecthub", false)
	}
	if strings.TrimSpace(hub.Spec.ApplicationsNamespace) == "" {
		return r.updateFailure(ctx, hub, "ApplicationsNamespaceRequired", "spec.applicationsNamespace must not be empty", false)
	}

	service := &dchv1alpha1.DataConnectService{}
	serviceKey := types.NamespacedName{Name: dataConnectServiceName, Namespace: hub.Spec.ApplicationsNamespace}
	if err := r.Get(ctx, serviceKey, service); err != nil && !apierrors.IsNotFound(err) {
		return ctrl.Result{}, fmt.Errorf("getting DataConnectService: %w", err)
	} else if err == nil && !ownedByHub(service, hub) {
		return r.updateFailure(ctx, hub, "OwnershipConflict", fmt.Sprintf(
			"DataConnectService %s/%s already exists and is not owned by this DataConnectHub; refusing to adopt it",
			service.Namespace, service.Name), true)
	}
	staleServices, err := r.deleteServicesOutsideNamespace(ctx, hub)
	if err != nil {
		return ctrl.Result{}, err
	}
	if staleServices {
		return r.updateWaiting(ctx, hub, "CleaningPreviousNamespace", "Waiting for the previously managed DataConnectService to finish cleanup", true)
	}

	platformConfig := &corev1.ConfigMap{}
	platformConfigKey := types.NamespacedName{Name: modulePlatformConfigName, Namespace: hub.Spec.ApplicationsNamespace}
	if err := r.Get(ctx, platformConfigKey, platformConfig); err != nil {
		if apierrors.IsNotFound(err) {
			return r.updateWaiting(ctx, hub, "WaitingForPlatformConfig", "Waiting for the platform to create odh-dataconnecthub-config", true)
		}
		return ctrl.Result{}, fmt.Errorf("getting platform configuration ConfigMap: %w", err)
	}

	if err := r.reconcileApplicationConfig(ctx, hub, platformConfig); err != nil {
		return ctrl.Result{}, err
	}
	if err := r.reconcileDataConnectService(ctx, hub, serviceKey); err != nil {
		return ctrl.Result{}, err
	}
	if err := r.Get(ctx, serviceKey, service); err != nil {
		return ctrl.Result{}, fmt.Errorf("reading reconciled DataConnectService: %w", err)
	}

	if err := r.updateStatus(ctx, hub, service); err != nil {
		return ctrl.Result{}, err
	}
	if isConditionTrue(service.Status.Conditions, conditionReady) {
		return ctrl.Result{RequeueAfter: 5 * time.Minute}, nil
	}
	log.Info("DataConnectHub is waiting for DataConnectService readiness", "namespace", service.Namespace)
	return ctrl.Result{RequeueAfter: reconcilePendingInterval}, nil
}

func (r *DataConnectHubReconciler) reconcileDataConnectService(
	ctx context.Context,
	hub *componentsplatformv1alpha1.DataConnectHub,
	key types.NamespacedName,
) error {
	service := &dchv1alpha1.DataConnectService{}
	err := r.Get(ctx, key, service)
	if apierrors.IsNotFound(err) {
		service = &dchv1alpha1.DataConnectService{
			ObjectMeta: metav1.ObjectMeta{Name: key.Name, Namespace: key.Namespace},
			Spec:       desiredDataConnectServiceSpec(hub),
		}
		if err := controllerutil.SetControllerReference(hub, service, r.Scheme); err != nil {
			return fmt.Errorf("setting DataConnectService owner reference: %w", err)
		}
		if err := r.Create(ctx, service); err != nil && !apierrors.IsAlreadyExists(err) {
			return fmt.Errorf("creating DataConnectService: %w", err)
		}
		return nil
	}
	if err != nil {
		return fmt.Errorf("getting DataConnectService: %w", err)
	}
	existing := service.DeepCopy()
	if !ownedByHub(service, hub) {
		return fmt.Errorf("DataConnectService %s/%s ownership changed during reconciliation", service.Namespace, service.Name)
	}

	desired := desiredDataConnectServiceSpec(hub)
	if !reflect.DeepEqual(service.Spec, desired) {
		service.Spec = desired
		if err := r.Patch(ctx, service, client.MergeFrom(existing)); err != nil {
			return fmt.Errorf("updating DataConnectService spec: %w", err)
		}
	}
	return nil
}

func desiredDataConnectServiceSpec(hub *componentsplatformv1alpha1.DataConnectHub) dchv1alpha1.DataConnectServiceSpec {
	var gateway *dchv1alpha1.Gateway
	if hub.Spec.Gateway != nil {
		copy := *hub.Spec.Gateway
		gateway = &copy
	}
	return dchv1alpha1.DataConnectServiceSpec{
		Gateway:              gateway,
		TokenReviewAudiences: append([]string(nil), hub.Spec.TokenReviewAudiences...),
	}
}

func (r *DataConnectHubReconciler) reconcileApplicationConfig(
	ctx context.Context,
	hub *componentsplatformv1alpha1.DataConnectHub,
	cm *corev1.ConfigMap,
) error {
	before := cm.DeepCopy()
	if cm.Data == nil {
		cm.Data = make(map[string]string)
	}
	if hub.Spec.Distribution.Name != "" {
		cm.Data[distributionNameKey] = hub.Spec.Distribution.Name
	}
	if hub.Spec.Distribution.Version != "" {
		cm.Data[distributionVersionKey] = hub.Spec.Distribution.Version
	}
	if hub.Spec.Gateway != nil {
		cm.Data["gateway.name"] = hub.Spec.Gateway.Name
		cm.Data["gateway.namespace"] = hub.Spec.Gateway.Namespace
	} else {
		delete(cm.Data, "gateway.name")
		delete(cm.Data, "gateway.namespace")
	}
	if hub.Spec.TokenReviewAudiences != nil {
		cm.Data["auth.tokenReviewAudiences"] = strings.Join(hub.Spec.TokenReviewAudiences, ",")
	} else {
		delete(cm.Data, "auth.tokenReviewAudiences")
	}
	if !reflect.DeepEqual(before.Data, cm.Data) {
		if err := r.Patch(ctx, cm, client.MergeFrom(before)); err != nil {
			return fmt.Errorf("updating platform configuration ConfigMap: %w", err)
		}
	}
	return nil
}

func (r *DataConnectHubReconciler) updateStatus(
	ctx context.Context,
	hub *componentsplatformv1alpha1.DataConnectHub,
	service *dchv1alpha1.DataConnectService,
) error {
	updated := hub.DeepCopy()
	updated.Status.ObservedGeneration = hub.Generation
	updated.Status.Distribution = service.Status.Distribution
	updated.Status.Releases = append([]dchv1alpha1.ReleaseStatus(nil), service.Status.Releases...)
	updated.Status.Conditions = nil

	ready := findCondition(service.Status.Conditions, conditionReady)
	provisioning := findCondition(service.Status.Conditions, conditionProvisioningSucceeded)
	degraded := findCondition(service.Status.Conditions, conditionDegraded)
	childCurrent := service.Status.ObservedGeneration >= service.Generation

	if !childCurrent || ready == nil {
		setHubCondition(updated, conditionReady, metav1.ConditionFalse, "WaitingForDataConnectService", "Waiting for DataConnectService to observe its desired generation")
		setHubCondition(updated, conditionProvisioningSucceeded, metav1.ConditionUnknown, "WaitingForDataConnectService", "DataConnectService has not reported provisioning status")
		setHubCondition(updated, conditionDegraded, metav1.ConditionFalse, "Reconciling", "No module-level errors")
	} else {
		copyChildCondition(updated, conditionReady, ready)
		if provisioning != nil {
			copyChildCondition(updated, conditionProvisioningSucceeded, provisioning)
		} else {
			setHubCondition(updated, conditionProvisioningSucceeded, metav1.ConditionUnknown, "ConditionUnavailable", "DataConnectService has not reported ProvisioningSucceeded")
		}
		if degraded != nil {
			copyChildCondition(updated, conditionDegraded, degraded)
		} else {
			setHubCondition(updated, conditionDegraded, metav1.ConditionFalse, "ConditionUnavailable", "DataConnectService has not reported degradation")
		}
	}

	if reflect.DeepEqual(hub.Status, updated.Status) {
		return nil
	}
	if err := r.Status().Patch(ctx, updated, client.MergeFrom(hub)); err != nil {
		return fmt.Errorf("updating DataConnectHub status: %w", err)
	}
	return nil
}

func (r *DataConnectHubReconciler) updateFailure(
	ctx context.Context,
	hub *componentsplatformv1alpha1.DataConnectHub,
	reason, message string,
	retry bool,
) (ctrl.Result, error) {
	updated := hub.DeepCopy()
	setHubCondition(updated, conditionReady, metav1.ConditionFalse, reason, message)
	setHubCondition(updated, conditionProvisioningSucceeded, metav1.ConditionFalse, reason, message)
	setHubCondition(updated, conditionDegraded, metav1.ConditionTrue, reason, message)
	if !reflect.DeepEqual(hub.Status, updated.Status) {
		if err := r.Status().Patch(ctx, updated, client.MergeFrom(hub)); err != nil {
			return ctrl.Result{}, fmt.Errorf("updating DataConnectHub failure status: %w", err)
		}
	}
	if retry {
		return ctrl.Result{RequeueAfter: reconcilePendingInterval}, nil
	}
	return ctrl.Result{}, nil
}

func (r *DataConnectHubReconciler) updateWaiting(
	ctx context.Context,
	hub *componentsplatformv1alpha1.DataConnectHub,
	reason, message string,
	retry bool,
) (ctrl.Result, error) {
	updated := hub.DeepCopy()
	setHubCondition(updated, conditionReady, metav1.ConditionFalse, reason, message)
	setHubCondition(updated, conditionProvisioningSucceeded, metav1.ConditionUnknown, reason, message)
	setHubCondition(updated, conditionDegraded, metav1.ConditionFalse, reason, "No reconciliation errors")
	if !reflect.DeepEqual(hub.Status, updated.Status) {
		if err := r.Status().Patch(ctx, updated, client.MergeFrom(hub)); err != nil {
			return ctrl.Result{}, fmt.Errorf("updating DataConnectHub waiting status: %w", err)
		}
	}
	if retry {
		return ctrl.Result{RequeueAfter: reconcilePendingInterval}, nil
	}
	return ctrl.Result{}, nil
}

func (r *DataConnectHubReconciler) reconcileDelete(ctx context.Context, hub *componentsplatformv1alpha1.DataConnectHub) (ctrl.Result, error) {
	if !controllerutil.ContainsFinalizer(hub, dataConnectHubFinalizer) {
		return ctrl.Result{}, nil
	}
	services := &dchv1alpha1.DataConnectServiceList{}
	if err := r.List(ctx, services); err != nil {
		return ctrl.Result{}, fmt.Errorf("listing DataConnectServices for cleanup: %w", err)
	}
	remaining := false
	for i := range services.Items {
		service := &services.Items[i]
		if !ownedByHub(service, hub) {
			continue
		}
		remaining = true
		if err := r.Delete(ctx, service); err != nil && !apierrors.IsNotFound(err) {
			return ctrl.Result{}, fmt.Errorf("deleting managed DataConnectService %s/%s: %w", service.Namespace, service.Name, err)
		}
	}
	if remaining {
		return ctrl.Result{RequeueAfter: reconcilePendingInterval}, nil
	}
	controllerutil.RemoveFinalizer(hub, dataConnectHubFinalizer)
	if err := r.Update(ctx, hub); err != nil {
		return ctrl.Result{}, fmt.Errorf("removing DataConnectHub finalizer: %w", err)
	}
	return ctrl.Result{}, nil
}

func (r *DataConnectHubReconciler) deleteServicesOutsideNamespace(
	ctx context.Context,
	hub *componentsplatformv1alpha1.DataConnectHub,
) (bool, error) {
	services := &dchv1alpha1.DataConnectServiceList{}
	if err := r.List(ctx, services); err != nil {
		return false, fmt.Errorf("listing DataConnectServices for namespace transition: %w", err)
	}
	found := false
	for i := range services.Items {
		service := &services.Items[i]
		if !ownedByHub(service, hub) || service.Namespace == hub.Spec.ApplicationsNamespace {
			continue
		}
		found = true
		if err := r.Delete(ctx, service); err != nil && !apierrors.IsNotFound(err) {
			return false, fmt.Errorf("deleting DataConnectService from previous namespace %s: %w", service.Namespace, err)
		}
	}
	return found, nil
}

func ownedByHub(obj metav1.Object, hub *componentsplatformv1alpha1.DataConnectHub) bool {
	owner := metav1.GetControllerOf(obj)
	return owner != nil && owner.UID == hub.UID && owner.Kind == dataConnectHubKind
}

func findCondition(conditions []metav1.Condition, conditionType string) *metav1.Condition {
	for i := range conditions {
		if conditions[i].Type == conditionType {
			return &conditions[i]
		}
	}
	return nil
}

func isConditionTrue(conditions []metav1.Condition, conditionType string) bool {
	condition := findCondition(conditions, conditionType)
	return condition != nil && condition.Status == metav1.ConditionTrue
}

func setHubCondition(hub *componentsplatformv1alpha1.DataConnectHub, conditionType string, status metav1.ConditionStatus, reason, message string) {
	meta.SetStatusCondition(&hub.Status.Conditions, metav1.Condition{
		Type:               conditionType,
		Status:             status,
		ObservedGeneration: hub.Generation,
		Reason:             reason,
		Message:            message,
	})
}

func copyChildCondition(hub *componentsplatformv1alpha1.DataConnectHub, conditionType string, child *metav1.Condition) {
	setHubCondition(hub, conditionType, child.Status, child.Reason, child.Message)
}

func (r *DataConnectHubReconciler) platformConfigToReconcile(ctx context.Context, obj client.Object) []reconcile.Request {
	var hubs componentsplatformv1alpha1.DataConnectHubList
	if err := r.List(ctx, &hubs); err != nil {
		return nil
	}
	requests := make([]reconcile.Request, 0, len(hubs.Items))
	for i := range hubs.Items {
		if hubs.Items[i].Spec.ApplicationsNamespace == obj.GetNamespace() {
			requests = append(requests, reconcile.Request{NamespacedName: types.NamespacedName{Name: hubs.Items[i].Name}})
		}
	}
	return requests
}

// SetupWithManager watches module configuration and the namespaced application
// CR so status and config changes propagate back to the platform module.
func (r *DataConnectHubReconciler) SetupWithManager(mgr ctrl.Manager) error {
	isPlatformConfig := predicate.NewPredicateFuncs(func(obj client.Object) bool {
		return obj.GetName() == modulePlatformConfigName
	})

	return ctrl.NewControllerManagedBy(mgr).
		For(&componentsplatformv1alpha1.DataConnectHub{}).
		Owns(&dchv1alpha1.DataConnectService{}).
		Watches(&corev1.ConfigMap{}, handler.EnqueueRequestsFromMapFunc(r.platformConfigToReconcile), builder.WithPredicates(isPlatformConfig)).
		Named("dataconnecthub").
		Complete(r)
}
