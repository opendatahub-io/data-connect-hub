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
	"path/filepath"
	"time"

	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	networkingv1 "k8s.io/api/networking/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/builder"
	"sigs.k8s.io/controller-runtime/pkg/client"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/predicate"

	dataconnecthubv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/v1alpha1"
)

const (
	defaultRestImage   = "ghcr.io/opendatahub-io/data-connect-hub/rest-service:latest"
	defaultFlightImage = "ghcr.io/opendatahub-io/data-connect-hub/flight-service:latest"

	defaultGatewayName      = "odh-gateway"
	defaultGatewayNamespace = "opendatahub"

	conditionTypeAvailable   = "Available"
	conditionTypeProgressing = "Progressing"
	conditionTypeDegraded    = "Degraded"

	requeueWaitingForReady = 10 * time.Second
	requeueOnError         = 30 * time.Second
	requeueWhenReady       = 5 * time.Minute

	nameRestService    = "rest-service"
	nameFlightService  = "flight-service"
	namePostgres       = "postgres"
	nameDataConnectHub = "data-connect-hub"
	namePostgresCreds  = "postgres-credentials"
)

// DataConnectServiceReconciler reconciles a DataConnectService object
type DataConnectServiceReconciler struct {
	client.Client
	Scheme        *runtime.Scheme
	ManifestsPath string
}

// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices/status,verbs=get;update;patch
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices/finalizers,verbs=update
// +kubebuilder:rbac:groups=apps,resources=deployments,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups="",resources=services;configmaps;secrets;serviceaccounts;persistentvolumeclaims,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=networking.k8s.io,resources=networkpolicies,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=gateway.networking.k8s.io,resources=httproutes,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=gateway.networking.k8s.io,resources=gateways,verbs=get;list;watch

func (r *DataConnectServiceReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	var cr dataconnecthubv1alpha1.DataConnectService
	if err := r.Get(ctx, req.NamespacedName, &cr); err != nil {
		if apierrors.IsNotFound(err) {
			return ctrl.Result{}, nil
		}
		return ctrl.Result{}, err
	}

	log.Info("reconciling DataConnectService", "name", cr.Name, "namespace", cr.Namespace)

	// Phase 1: Database (postgres first, services depend on it)
	devMode := cr.Spec.Database == nil || cr.Spec.Database.DevMode == nil || *cr.Spec.Database.DevMode
	if devMode {
		if err := r.reconcileDatabase(ctx, &cr); err != nil {
			log.Error(err, "failed to reconcile Database")
			return r.updateStatus(ctx, req, "Error", func(cr *dataconnecthubv1alpha1.DataConnectService) {
				r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "DatabaseError", err.Error())
				r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "DatabaseError", err.Error())
				r.setCondition(cr, conditionTypeProgressing, metav1.ConditionFalse, "DatabaseError", "Reconciliation failed")
			})
		}
		if pgReady, err := r.isDeploymentReady(ctx, cr.Namespace, namePostgres); err != nil {
			log.Error(err, "failed to check postgres readiness")
			return r.updateStatus(ctx, req, "Error", func(cr *dataconnecthubv1alpha1.DataConnectService) {
				r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "DatabaseError", err.Error())
				r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "DatabaseError", err.Error())
				r.setCondition(cr, conditionTypeProgressing, metav1.ConditionFalse, "DatabaseError", "Reconciliation failed")
			})
		} else if !pgReady {
			log.Info("waiting for postgres to become ready")
			return r.updateStatus(ctx, req, "Progressing", func(cr *dataconnecthubv1alpha1.DataConnectService) {
				r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "WaitingForDatabase", "Waiting for postgres deployment to become ready")
				r.setCondition(cr, conditionTypeProgressing, metav1.ConditionTrue, "WaitingForDatabase", "Waiting for postgres deployment to become ready")
				r.setCondition(cr, conditionTypeDegraded, metav1.ConditionFalse, "WaitingForDatabase", "No errors")
			})
		}
	}

	// Phase 2: Services (only after database is ready)
	if err := r.reconcileService(ctx, &cr, nameRestService, cr.Spec.RestService); err != nil {
		log.Error(err, "failed to reconcile RestService")
		return r.updateStatus(ctx, req, "Error", func(cr *dataconnecthubv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "RestServiceError", err.Error())
			r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "RestServiceError", err.Error())
			r.setCondition(cr, conditionTypeProgressing, metav1.ConditionFalse, "RestServiceError", "Reconciliation failed")
		})
	}

	if err := r.reconcileService(ctx, &cr, nameFlightService, cr.Spec.FlightService); err != nil {
		log.Error(err, "failed to reconcile FlightService")
		return r.updateStatus(ctx, req, "Error", func(cr *dataconnecthubv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "FlightServiceError", err.Error())
			r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "FlightServiceError", err.Error())
			r.setCondition(cr, conditionTypeProgressing, metav1.ConditionFalse, "FlightServiceError", "Reconciliation failed")
		})
	}

	// Phase 3: Gateway (skipped if Gateway API CRDs are not installed)
	if err := r.reconcileHTTPRoute(ctx, &cr); err != nil {
		if meta.IsNoMatchError(err) {
			log.Info("Gateway API CRDs not installed, skipping HTTPRoute creation")
		} else {
			log.Error(err, "failed to reconcile HTTPRoute")
			return r.updateStatus(ctx, req, "Error", func(cr *dataconnecthubv1alpha1.DataConnectService) {
				r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "HTTPRouteError", err.Error())
				r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "HTTPRouteError", err.Error())
				r.setCondition(cr, conditionTypeProgressing, metav1.ConditionFalse, "HTTPRouteError", "Reconciliation failed")
			})
		}
	}

	// Phase 4: Check all deployments are ready before declaring Available
	pendingDeployments, err := r.pendingDeployments(ctx, cr.Namespace, devMode)
	if err != nil {
		log.Error(err, "failed to check deployment readiness")
		return r.updateStatus(ctx, req, "Error", func(cr *dataconnecthubv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "DeploymentCheckError", err.Error())
			r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "DeploymentCheckError", err.Error())
			r.setCondition(cr, conditionTypeProgressing, metav1.ConditionFalse, "DeploymentCheckError", "Reconciliation failed")
		})
	}
	if len(pendingDeployments) > 0 {
		msg := fmt.Sprintf("Waiting for deployments: %v", pendingDeployments)
		log.Info(msg)
		return r.updateStatus(ctx, req, "Progressing", func(cr *dataconnecthubv1alpha1.DataConnectService) {
			r.gatewayStatus(ctx, cr)
			r.setCondition(cr, conditionTypeAvailable, metav1.ConditionFalse, "WaitingForDeployments", msg)
			r.setCondition(cr, conditionTypeProgressing, metav1.ConditionTrue, "WaitingForDeployments", msg)
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionFalse, "WaitingForDeployments", "No errors")
		})
	}

	// All ready
	return r.updateStatus(ctx, req, "Ready", func(cr *dataconnecthubv1alpha1.DataConnectService) {
		r.gatewayStatus(ctx, cr)
		r.setCondition(cr, conditionTypeAvailable, metav1.ConditionTrue, "Reconciled", "All resources reconciled and ready")
		r.setCondition(cr, conditionTypeProgressing, metav1.ConditionFalse, "Reconciled", "Reconciliation complete")
		r.setCondition(cr, conditionTypeDegraded, metav1.ConditionFalse, "Reconciled", "No errors")
	})
}

func (r *DataConnectServiceReconciler) updateStatus(
	ctx context.Context,
	req ctrl.Request,
	phase string,
	mutate func(*dataconnecthubv1alpha1.DataConnectService),
) (ctrl.Result, error) {
	var cr dataconnecthubv1alpha1.DataConnectService
	if err := r.Get(ctx, req.NamespacedName, &cr); err != nil {
		return ctrl.Result{}, err
	}

	cr.Status.Phase = phase
	mutate(&cr)

	if err := r.Status().Update(ctx, &cr); err != nil {
		if apierrors.IsConflict(err) {
			return ctrl.Result{Requeue: true}, nil
		}
		return ctrl.Result{}, err
	}

	if phase == "Ready" {
		return ctrl.Result{RequeueAfter: requeueWhenReady}, nil
	}
	if phase == "Error" {
		return ctrl.Result{RequeueAfter: requeueOnError}, nil
	}
	return ctrl.Result{RequeueAfter: requeueWaitingForReady}, nil
}

func (r *DataConnectServiceReconciler) reconcileDatabase(ctx context.Context, cr *dataconnecthubv1alpha1.DataConnectService) error {
	if err := r.reconcilePostgresSecret(ctx, cr); err != nil {
		return fmt.Errorf("postgres secret: %w", err)
	}

	pgPath := filepath.Join(r.ManifestsPath, "db", "postgres")
	resources, err := renderPostgresKustomization(pgPath)
	if err != nil {
		return fmt.Errorf("rendering postgres manifests: %w", err)
	}

	return r.applyResources(ctx, cr, resources)
}

func (r *DataConnectServiceReconciler) reconcileService(
	ctx context.Context,
	cr *dataconnecthubv1alpha1.DataConnectService,
	name string,
	overrides *dataconnecthubv1alpha1.ServiceOverrides,
) error {
	basePath := filepath.Join(r.ManifestsPath, "base", name)

	patches, images := buildServicePatches(name, overrides)

	resources, err := renderKustomization(basePath, patches, images)
	if err != nil {
		return fmt.Errorf("rendering %s manifests: %w", name, err)
	}

	return r.applyResources(ctx, cr, resources)
}

func (r *DataConnectServiceReconciler) reconcileHTTPRoute(ctx context.Context, cr *dataconnecthubv1alpha1.DataConnectService) error {
	gwPath := filepath.Join(r.ManifestsPath, "gateway")

	patches := buildGatewayPatches(cr.Spec.Gateway)

	resources, err := renderKustomization(gwPath, patches, nil)
	if err != nil {
		return fmt.Errorf("rendering gateway manifests: %w", err)
	}

	return r.applyResources(ctx, cr, resources)
}

func (r *DataConnectServiceReconciler) gatewayStatus(ctx context.Context, cr *dataconnecthubv1alpha1.DataConnectService) {
	gwName := defaultGatewayName
	gwNamespace := defaultGatewayNamespace
	if cr.Spec.Gateway != nil {
		gwName = cr.Spec.Gateway.Name
		gwNamespace = cr.Spec.Gateway.Namespace
	}
	cr.Status.HttpRoute = nameDataConnectHub
	cr.Status.Gateway = &dataconnecthubv1alpha1.Gateway{
		Name:      gwName,
		Namespace: gwNamespace,
	}

	cr.Status.Hostname = r.resolveGatewayHostname(ctx, gwNamespace, gwName)
}

func (r *DataConnectServiceReconciler) resolveGatewayHostname(ctx context.Context, namespace, name string) string {
	gw := &unstructured.Unstructured{}
	gw.SetGroupVersionKind(schema.GroupVersionKind{
		Group:   "gateway.networking.k8s.io",
		Version: "v1",
		Kind:    "Gateway",
	})
	if err := r.Get(ctx, types.NamespacedName{Name: name, Namespace: namespace}, gw); err != nil {
		return ""
	}

	addresses, found, _ := unstructured.NestedSlice(gw.Object, "status", "addresses")
	if !found || len(addresses) == 0 {
		return ""
	}
	if addr, ok := addresses[0].(map[string]any); ok {
		if val, ok := addr["value"].(string); ok {
			return val
		}
	}
	return ""
}

// isDeploymentReady checks if a deployment has all replicas available.
func (r *DataConnectServiceReconciler) isDeploymentReady(ctx context.Context, namespace, name string) (bool, error) {
	deploy := &appsv1.Deployment{}
	if err := r.Get(ctx, types.NamespacedName{Name: name, Namespace: namespace}, deploy); err != nil {
		if apierrors.IsNotFound(err) {
			return false, nil
		}
		return false, err
	}
	ready := deploy.Status.ReadyReplicas == deploy.Status.Replicas &&
		deploy.Status.UpdatedReplicas == deploy.Status.Replicas &&
		deploy.Generation == deploy.Status.ObservedGeneration
	return ready, nil
}

// pendingDeployments returns the names of deployments that are not yet ready.
func (r *DataConnectServiceReconciler) pendingDeployments(ctx context.Context, namespace string, includePostgres bool) ([]string, error) {
	names := []string{nameRestService, nameFlightService}
	if includePostgres {
		names = append(names, namePostgres)
	}
	var pending []string
	for _, name := range names {
		ready, err := r.isDeploymentReady(ctx, namespace, name)
		if err != nil {
			return nil, fmt.Errorf("checking deployment %s: %w", name, err)
		}
		if !ready {
			pending = append(pending, name)
		}
	}
	return pending, nil
}

func (r *DataConnectServiceReconciler) setCondition(cr *dataconnecthubv1alpha1.DataConnectService, condType string, status metav1.ConditionStatus, reason, message string) {
	meta.SetStatusCondition(&cr.Status.Conditions, metav1.Condition{
		Type:               condType,
		Status:             status,
		ObservedGeneration: cr.Generation,
		Reason:             reason,
		Message:            message,
	})
}

// SetupWithManager sets up the controller with the Manager.
func (r *DataConnectServiceReconciler) SetupWithManager(mgr ctrl.Manager) error {
	// Matches the DSCInitialization controller pattern: react to spec (generation) and label
	// changes but ignore platform-driven metadata updates (e.g. OpenShift's
	// image-registry-pull-secrets controller continuously SSA-applies to ServiceAccounts).
	ownsPredicate := predicate.Or(predicate.GenerationChangedPredicate{}, predicate.LabelChangedPredicate{})

	return ctrl.NewControllerManagedBy(mgr).
		For(&dataconnecthubv1alpha1.DataConnectService{}, builder.WithPredicates(predicate.GenerationChangedPredicate{})).
		Owns(&appsv1.Deployment{}).
		Owns(&corev1.Service{}, builder.WithPredicates(ownsPredicate)).
		Owns(&corev1.ConfigMap{}, builder.WithPredicates(ownsPredicate)).
		Owns(&corev1.ServiceAccount{}, builder.WithPredicates(ownsPredicate)).
		Owns(&corev1.PersistentVolumeClaim{}, builder.WithPredicates(ownsPredicate)).
		Owns(&corev1.Secret{}, builder.WithPredicates(ownsPredicate)).
		Owns(&networkingv1.NetworkPolicy{}, builder.WithPredicates(ownsPredicate)).
		Named("dataconnectservice").
		Complete(r)
}
