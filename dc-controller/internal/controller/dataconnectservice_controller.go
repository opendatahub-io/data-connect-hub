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
	"os"
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
	"sigs.k8s.io/controller-runtime/pkg/controller/controllerutil"
	"sigs.k8s.io/controller-runtime/pkg/handler"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/predicate"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	defaultGatewayName      = "odh-gateway"
	defaultGatewayNamespace = "opendatahub"
	defaultNamespace        = "opendatahub"

	conditionTypeReady                 = "Ready"
	conditionTypeProvisioningSucceeded = "ProvisioningSucceeded"
	conditionTypeDegraded              = "Degraded"

	requeueWaitingForReady = 10 * time.Second
	requeueOnError         = 30 * time.Second
	requeueWhenReady       = 5 * time.Minute

	nameRestService    = "rest-service"
	nameFlightService  = "flight-service"
	nameDataConnectHub = "data-connect-hub"
	nameDatabaseConfig = "dch-database-config"

	kindDeployment = "Deployment"

	repoURL = "https://github.com/opendatahub-io/data-connect-hub"

	platformConfigName = "opendatahub-dataconnecthub-config"

	finalizerName = "dataconnecthub.opendatahub.io/finalizer"

	singletonCRName = "default-dataconnectservice"

	releasePlatform = "platform"
)

// BuildVersion is set at build time via -ldflags.
var BuildVersion = "dev"

// DataConnectServiceReconciler reconciles a DataConnectService object
type DataConnectServiceReconciler struct {
	client.Client
	Scheme             *runtime.Scheme
	ManifestsPath      string
	Namespace          string // target namespace for deploying workloads
	RestImage          string // resolved from RELATED_IMAGE or default
	FlightImage        string // resolved from RELATED_IMAGE or default
	KubeRbacProxyImage string // resolved from RELATED_IMAGE or default
}

type platformConfig struct {
	Distribution     dchv1alpha1.DistributionStatus
	PlatformVersion  string
	GatewayName      string
	GatewayNamespace string
}

func (r *DataConnectServiceReconciler) readPlatformConfig(ctx context.Context) platformConfig {
	cfg := platformConfig{
		Distribution: dchv1alpha1.DistributionStatus{
			Name:    "Standalone",
			Version: BuildVersion,
		},
		GatewayName:      defaultGatewayName,
		GatewayNamespace: defaultGatewayNamespace,
	}

	cm := &corev1.ConfigMap{}
	if err := r.Get(ctx, types.NamespacedName{Name: platformConfigName, Namespace: r.Namespace}, cm); err != nil {
		if !apierrors.IsNotFound(err) {
			logf.FromContext(ctx).Error(err, "failed to read platform ConfigMap, using defaults")
		}
		return cfg
	}

	if v := cm.Data["distribution.name"]; v != "" {
		cfg.Distribution.Name = v
	}
	if v := cm.Data["distribution.version"]; v != "" {
		cfg.Distribution.Version = v
	}
	cfg.PlatformVersion = cm.Data["platformVersion"]
	if v := cm.Data["gateway.name"]; v != "" {
		cfg.GatewayName = v
	}
	if v := cm.Data["gateway.namespace"]; v != "" {
		cfg.GatewayNamespace = v
	}

	return cfg
}

// EnvOrDefault returns the value of the named environment variable or the fallback.
func EnvOrDefault(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices/status,verbs=get;update;patch
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=dataconnectservices/finalizers,verbs=update
// +kubebuilder:rbac:groups=dataconnecthub.opendatahub.io,resources=data-connections;data-connection-types,verbs=get;list;watch;create;update;patch;delete;post;put
// +kubebuilder:rbac:groups=apps,resources=deployments,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups="",resources=services;configmaps;serviceaccounts,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups="",resources=secrets,verbs=get;list;watch;update;patch
// +kubebuilder:rbac:groups=networking.k8s.io,resources=networkpolicies,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=gateway.networking.k8s.io,resources=httproutes,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=gateway.networking.k8s.io,resources=gateways,verbs=get;list;watch
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=clusterroles;clusterrolebindings,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=authentication.k8s.io,resources=tokenreviews,verbs=create
// +kubebuilder:rbac:groups=authorization.k8s.io,resources=subjectaccessreviews,verbs=create

func (r *DataConnectServiceReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	// Only reconcile CRs in the operand namespace
	if req.Namespace != r.Namespace {
		log.Info("ignoring DataConnectService outside operand namespace", "namespace", req.Namespace)
		return ctrl.Result{}, nil
	}

	var cr dchv1alpha1.DataConnectService
	if err := r.Get(ctx, req.NamespacedName, &cr); err != nil {
		if apierrors.IsNotFound(err) {
			return ctrl.Result{}, nil
		}
		return ctrl.Result{}, err
	}

	// Handle deletion — run finalizer then allow GC
	if !cr.DeletionTimestamp.IsZero() {
		if controllerutil.ContainsFinalizer(&cr, finalizerName) {
			log.Info("running finalizer for DataConnectService")
			controllerutil.RemoveFinalizer(&cr, finalizerName)
			return ctrl.Result{}, r.Update(ctx, &cr)
		}
		return ctrl.Result{}, nil
	}

	// Ensure finalizer is present
	if !controllerutil.ContainsFinalizer(&cr, finalizerName) {
		controllerutil.AddFinalizer(&cr, finalizerName)
		if err := r.Update(ctx, &cr); err != nil {
			return ctrl.Result{}, err
		}
	}

	log.Info("reconciling DataConnectService", "name", cr.Name)

	// Read platform configuration from ConfigMap
	platCfg := r.readPlatformConfig(ctx)

	// Phase 1: Validate database secret exists
	if err := r.validateDatabaseSecret(ctx); err != nil {
		log.Error(err, "database secret validation failed")
		return r.updateStatus(ctx, req, &platCfg, "Error", func(cr *dchv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "DatabaseSecretMissing", err.Error())
			r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "DatabaseSecretMissing", err.Error())
			r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionFalse, "DatabaseSecretMissing",
				"Secret 'dch-database-config' with keys DATABASE_URL and secret-config.toml is required")
		})
	}

	// Phase 2: Services (only after database is ready)
	if err := r.reconcileService(ctx, &cr, nameRestService, cr.Spec.RestService); err != nil {
		log.Error(err, "failed to reconcile RestService")
		return r.updateStatus(ctx, req, &platCfg, "Error", func(cr *dchv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "RestServiceError", err.Error())
			r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "RestServiceError", err.Error())
			r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionFalse, "RestServiceError", "Failed to apply rest-service manifests")
		})
	}

	if err := r.reconcileService(ctx, &cr, nameFlightService, cr.Spec.FlightService); err != nil {
		log.Error(err, "failed to reconcile FlightService")
		return r.updateStatus(ctx, req, &platCfg, "Error", func(cr *dchv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "FlightServiceError", err.Error())
			r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "FlightServiceError", err.Error())
			r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionFalse, "FlightServiceError", "Failed to apply flight-service manifests")
		})
	}

	// Phase 3: Gateway (skipped if Gateway API CRDs are not installed)
	if err := r.reconcileHTTPRoute(ctx, &cr, &platCfg); err != nil {
		if meta.IsNoMatchError(err) {
			log.Info("Gateway API CRDs not installed, skipping HTTPRoute creation")
		} else {
			log.Error(err, "failed to reconcile HTTPRoute")
			return r.updateStatus(ctx, req, &platCfg, "Error", func(cr *dchv1alpha1.DataConnectService) {
				r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "HTTPRouteError", err.Error())
				r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "HTTPRouteError", err.Error())
				r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionFalse, "HTTPRouteError", "Failed to apply gateway manifests")
			})
		}
	}

	// All manifests applied successfully
	// Phase 4: Check all deployments are ready before declaring Ready
	pendingDeployments, err := r.pendingDeployments(ctx, r.Namespace, cr.UID)
	if err != nil {
		log.Error(err, "failed to check deployment readiness")
		return r.updateStatus(ctx, req, &platCfg, "Error", func(cr *dchv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "DeploymentCheckError", err.Error())
			r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "DeploymentCheckError", err.Error())
			r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionTrue, "ProvisioningComplete", "Manifests applied successfully")
		})
	}
	if len(pendingDeployments) > 0 {
		msg := fmt.Sprintf("Waiting for deployments: %v", pendingDeployments)
		log.Info(msg)
		return r.updateStatus(ctx, req, &platCfg, "Progressing", func(cr *dchv1alpha1.DataConnectService) {
			r.gatewayStatus(ctx, cr, &platCfg)
			r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "WaitingForDeployments", msg)
			r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionTrue, "ProvisioningComplete", "Manifests applied successfully")
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionFalse, "WaitingForDeployments", "No errors")
		})
	}

	// All ready
	return r.updateStatus(ctx, req, &platCfg, "Ready", func(cr *dchv1alpha1.DataConnectService) {
		r.gatewayStatus(ctx, cr, &platCfg)
		r.setCondition(cr, conditionTypeReady, metav1.ConditionTrue, "Ready", "All resources reconciled and ready")
		r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionTrue, "ProvisioningComplete", "Manifests applied successfully")
		r.setCondition(cr, conditionTypeDegraded, metav1.ConditionFalse, "Reconciled", "No errors")
	})
}

func (r *DataConnectServiceReconciler) updateStatus(
	ctx context.Context,
	req ctrl.Request,
	platCfg *platformConfig,
	phase string,
	mutate func(*dchv1alpha1.DataConnectService),
) (ctrl.Result, error) {
	var cr dchv1alpha1.DataConnectService
	if err := r.Get(ctx, req.NamespacedName, &cr); err != nil {
		return ctrl.Result{}, err
	}

	cr.Status.Phase = phase
	cr.Status.ObservedGeneration = cr.Generation
	cr.Status.Distribution = platCfg.Distribution
	cr.Status.Releases = r.buildReleases(&cr, platCfg, phase == conditionTypeReady)
	mutate(&cr)

	if err := r.Status().Update(ctx, &cr); err != nil {
		if apierrors.IsConflict(err) {
			return ctrl.Result{Requeue: true}, nil
		}
		return ctrl.Result{}, err
	}

	if phase == conditionTypeReady {
		return ctrl.Result{RequeueAfter: requeueWhenReady}, nil
	}
	if phase == "Error" {
		return ctrl.Result{RequeueAfter: requeueOnError}, nil
	}
	return ctrl.Result{RequeueAfter: requeueWaitingForReady}, nil
}

// buildReleases constructs the status.releases list.
// The platform version entry is only advanced when the module is Ready,
// implementing the v2 platform version handshake protocol.
func (r *DataConnectServiceReconciler) buildReleases(
	cr *dchv1alpha1.DataConnectService,
	platCfg *platformConfig,
	isReady bool,
) []dchv1alpha1.ReleaseStatus {
	releases := make([]dchv1alpha1.ReleaseStatus, 2, 3)
	releases[0] = dchv1alpha1.ReleaseStatus{Name: "rest-service", RepoUrl: repoURL, Version: BuildVersion}
	releases[1] = dchv1alpha1.ReleaseStatus{Name: "flight-service", RepoUrl: repoURL, Version: BuildVersion}

	if platCfg.PlatformVersion == "" {
		return releases
	}

	platformRelease := dchv1alpha1.ReleaseStatus{
		Name: releasePlatform,
	}

	if isReady {
		platformRelease.Version = platCfg.PlatformVersion
	} else {
		for _, r := range cr.Status.Releases {
			if r.Name == releasePlatform {
				platformRelease.Version = r.Version
				break
			}
		}
	}

	return append(releases, platformRelease)
}

func (r *DataConnectServiceReconciler) reconcileService(
	ctx context.Context,
	cr *dchv1alpha1.DataConnectService,
	name string,
	overrides *dchv1alpha1.ServiceOverrides,
) error {
	basePath := filepath.Join(r.ManifestsPath, "base", name)

	patches := buildServicePatches(name, overrides)

	resources, err := renderKustomization(basePath, patches, nil)
	if err != nil {
		return fmt.Errorf("rendering %s manifests: %w", name, err)
	}

	image := resolveServiceImage(name, overrides, r.RestImage, r.FlightImage)
	setDeploymentImage(resources, name, image)
	if name == nameRestService {
		setDeploymentImage(resources, "kube-rbac-proxy", r.KubeRbacProxyImage)
	}
	setConfigMapGlobalNamespace(resources, name+"-config", r.Namespace)

	return r.applyResources(ctx, cr, resources)
}

func (r *DataConnectServiceReconciler) reconcileHTTPRoute(ctx context.Context, cr *dchv1alpha1.DataConnectService, platCfg *platformConfig) error {
	gwPath := filepath.Join(r.ManifestsPath, "gateway")

	gw := r.resolveGateway(cr, platCfg)
	patches := buildGatewayPatches(&gw)

	resources, err := renderKustomization(gwPath, patches, nil)
	if err != nil {
		return fmt.Errorf("rendering gateway manifests: %w", err)
	}

	return r.applyResources(ctx, cr, resources)
}

// resolveGateway merges gateway config: CR spec overrides ConfigMap, which overrides hardcoded defaults.
func (r *DataConnectServiceReconciler) resolveGateway(cr *dchv1alpha1.DataConnectService, platCfg *platformConfig) dchv1alpha1.Gateway {
	gw := dchv1alpha1.Gateway{
		Name:      platCfg.GatewayName,
		Namespace: platCfg.GatewayNamespace,
	}
	if cr.Spec.Gateway != nil {
		gw.Name = cr.Spec.Gateway.Name
		gw.Namespace = cr.Spec.Gateway.Namespace
	}
	return gw
}

func (r *DataConnectServiceReconciler) gatewayStatus(ctx context.Context, cr *dchv1alpha1.DataConnectService, platCfg *platformConfig) {
	gw := r.resolveGateway(cr, platCfg)
	cr.Status.HttpRoute = nameDataConnectHub
	cr.Status.Gateway = &dchv1alpha1.Gateway{
		Name:      gw.Name,
		Namespace: gw.Namespace,
	}
	cr.Status.Hostname = r.resolveGatewayHostname(ctx, gw.Namespace, gw.Name)
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

// pendingDeployments returns the names of managed deployments that are not yet ready.
func (r *DataConnectServiceReconciler) pendingDeployments(ctx context.Context, namespace string, ownerUID types.UID) ([]string, error) {
	deployList := &appsv1.DeploymentList{}
	if err := r.List(ctx, deployList,
		client.InNamespace(namespace),
		client.MatchingLabels{"dataconnecthub.opendatahub.io/managed-by": "dataconnectservice"},
	); err != nil {
		return nil, fmt.Errorf("listing managed deployments: %w", err)
	}

	var pending []string
	for i := range deployList.Items {
		d := &deployList.Items[i]
		if !isOwnedBy(d, ownerUID) {
			continue
		}
		ready := d.Status.ReadyReplicas == d.Status.Replicas &&
			d.Status.UpdatedReplicas == d.Status.Replicas &&
			d.Generation == d.Status.ObservedGeneration
		if !ready {
			pending = append(pending, d.Name)
		}
	}
	return pending, nil
}

func isOwnedBy(obj metav1.ObjectMetaAccessor, uid types.UID) bool {
	for _, ref := range obj.GetObjectMeta().GetOwnerReferences() {
		if ref.UID == uid {
			return true
		}
	}
	return false
}

func (r *DataConnectServiceReconciler) setCondition(cr *dchv1alpha1.DataConnectService, condType string, status metav1.ConditionStatus, reason, message string) {
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
	ownsPredicate := predicate.Or(predicate.GenerationChangedPredicate{}, predicate.LabelChangedPredicate{})

	isPlatformConfig := predicate.NewPredicateFuncs(func(obj client.Object) bool {
		return obj.GetName() == platformConfigName
	})

	return ctrl.NewControllerManagedBy(mgr).
		For(&dchv1alpha1.DataConnectService{}, builder.WithPredicates(predicate.GenerationChangedPredicate{})).
		Owns(&appsv1.Deployment{}, builder.WithPredicates(ownsPredicate)).
		Owns(&corev1.Service{}, builder.WithPredicates(ownsPredicate)).
		Owns(&corev1.ConfigMap{}, builder.WithPredicates(ownsPredicate)).
		Owns(&corev1.ServiceAccount{}, builder.WithPredicates(ownsPredicate)).
		Owns(&networkingv1.NetworkPolicy{}, builder.WithPredicates(ownsPredicate)).
		Watches(
			&corev1.ConfigMap{},
			handler.EnqueueRequestsFromMapFunc(r.platformConfigToReconcile),
			builder.WithPredicates(isPlatformConfig),
		).
		Named("dataconnectservice").
		Complete(r)
}

func (r *DataConnectServiceReconciler) platformConfigToReconcile(_ context.Context, _ client.Object) []reconcile.Request {
	return []reconcile.Request{{NamespacedName: types.NamespacedName{Name: singletonCRName, Namespace: r.Namespace}}}
}
