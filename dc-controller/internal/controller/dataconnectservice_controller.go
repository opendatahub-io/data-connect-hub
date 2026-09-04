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
	"strings"
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
	kustypes "sigs.k8s.io/kustomize/api/types"
	"sigs.k8s.io/yaml"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const (
	defaultGatewayName      = "odh-gateway"
	defaultGatewayNamespace = "opendatahub"

	conditionTypeReady                 = "Ready"
	conditionTypeProvisioningSucceeded = "ProvisioningSucceeded"
	conditionTypeDegraded              = "Degraded"
	conditionTypeGRPCGatewaySupported  = "GRPCGatewaySupported"

	http2EnableAnnotation = "ingress.operator.openshift.io/default-enable-http2"

	requeueWaitingForReady = 10 * time.Second
	requeueOnError         = 30 * time.Second
	requeueWhenReady       = 5 * time.Minute

	nameRestService    = "rest-service"
	nameFlightService  = "flight-service"
	nameDataConnectHub = "data-connect-hub"
	nameDatabaseConfig = "dch-database-config"

	kindDeployment = "Deployment"
	kindConfigMap  = "ConfigMap"

	repoURL = "https://github.com/opendatahub-io/data-connect-hub"

	platformConfigName = "opendatahub-dataconnecthub-config"

	finalizerName = "dataconnecthub.opendatahub.io/finalizer"

	releasePlatform = "platform"
)

// BuildVersion is set at build time via -ldflags.
var BuildVersion = "dev"

// DataConnectServiceReconciler reconciles a DataConnectService object
type DataConnectServiceReconciler struct {
	client.Client
	Scheme             *runtime.Scheme
	ManifestsPath      string
	RestImage          string
	FlightImage        string
	KubeRbacProxyImage string
}

type platformConfig struct {
	Distribution         dchv1alpha1.DistributionStatus
	PlatformVersion      string
	GatewayName          string
	GatewayNamespace     string
	TokenReviewAudiences []string
}

// readPlatformConfig reads cluster-level defaults from the platform ConfigMap.
// TODO(DSC): When DCH is onboarded to DSC, the platform operator will create
// and manage this ConfigMap (including auth.tokenReviewAudiences) in the operand
// namespace. Until then, standalone users create it manually or use CR overrides.
func (r *DataConnectServiceReconciler) readPlatformConfig(ctx context.Context, namespace string) platformConfig {
	cfg := platformConfig{
		Distribution: dchv1alpha1.DistributionStatus{
			Name:    "Standalone",
			Version: BuildVersion,
		},
		GatewayName:      defaultGatewayName,
		GatewayNamespace: defaultGatewayNamespace,
	}

	cm := &corev1.ConfigMap{}
	if err := r.Get(ctx, types.NamespacedName{Name: platformConfigName, Namespace: namespace}, cm); err != nil {
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
	if v := cm.Data["auth.tokenReviewAudiences"]; v != "" {
		var audiences []string
		for a := range strings.SplitSeq(v, ",") {
			if trimmed := strings.TrimSpace(a); trimmed != "" {
				audiences = append(audiences, trimmed)
			}
		}
		cfg.TokenReviewAudiences = audiences
	}

	return cfg
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
// +kubebuilder:rbac:groups=config.openshift.io,resources=ingresses,verbs=get;list;watch
// +kubebuilder:rbac:groups=operator.openshift.io,resources=ingresscontrollers,verbs=get;list;watch
// +kubebuilder:rbac:groups=rbac.authorization.k8s.io,resources=clusterroles;clusterrolebindings,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=authentication.k8s.io,resources=tokenreviews,verbs=create
// +kubebuilder:rbac:groups=authorization.k8s.io,resources=subjectaccessreviews,verbs=create

func (r *DataConnectServiceReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

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
			r.clearSyncedAnnotations(ctx)
			r.deleteInitDataConnectionTypes(ctx, cr.Namespace)
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

	log.Info("reconciling DataConnectService", "name", cr.Name, "namespace", cr.Namespace)

	// Read platform configuration from ConfigMap in the CR's namespace
	platCfg := r.readPlatformConfig(ctx, cr.Namespace)

	// Phase 1: Validate database secret exists
	if err := r.validateDatabaseSecret(ctx, cr.Namespace); err != nil {
		log.Error(err, "database secret validation failed")
		return r.updateStatus(ctx, req, &platCfg, "Error", func(cr *dchv1alpha1.DataConnectService) {
			r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "DatabaseSecretMissing", err.Error())
			r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "DatabaseSecretMissing", err.Error())
			r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionFalse, "DatabaseSecretMissing",
				"Secret 'dch-database-config' with key secret-config.toml is required")
		})
	}

	// Phase 2: Render and apply all manifests (services + gateway)
	if err := r.reconcileManifests(ctx, &cr, &platCfg); err != nil {
		if meta.IsNoMatchError(err) {
			log.Info("Gateway API CRDs not installed, skipping HTTPRoute creation")
		} else {
			log.Error(err, "failed to reconcile manifests")
			return r.updateStatus(ctx, req, &platCfg, "Error", func(cr *dchv1alpha1.DataConnectService) {
				r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "ManifestError", err.Error())
				r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "ManifestError", err.Error())
				r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionFalse, "ManifestError", "Failed to apply manifests")
			})
		}
	}

	// Phase 3: Ensure InitDataConnectionType CRs exist in the DCS namespace
	r.ensureInitDataConnectionTypes(ctx, &cr)

	// Phase 4: Check all deployments are ready before declaring Ready
	pendingDeployments, err := r.pendingDeployments(ctx, cr.Namespace, cr.UID)
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
			r.checkGRPCGatewaySupport(ctx, cr)
		})
	}

	// All ready
	return r.updateStatus(ctx, req, &platCfg, "Ready", func(cr *dchv1alpha1.DataConnectService) {
		r.gatewayStatus(ctx, cr, &platCfg)
		r.setCondition(cr, conditionTypeReady, metav1.ConditionTrue, "Ready", "All resources reconciled and ready")
		r.setCondition(cr, conditionTypeProvisioningSucceeded, metav1.ConditionTrue, "ProvisioningComplete", "Manifests applied successfully")
		r.setCondition(cr, conditionTypeDegraded, metav1.ConditionFalse, "Reconciled", "No errors")
		r.checkGRPCGatewaySupport(ctx, cr)
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

func (r *DataConnectServiceReconciler) reconcileManifests(
	ctx context.Context,
	cr *dchv1alpha1.DataConnectService,
	platCfg *platformConfig,
) error {
	basePath := filepath.Join(r.ManifestsPath, "base")

	gw := r.resolveGateway(cr, platCfg)
	restPatches := buildServicePatches(nameRestService, cr.Spec.RestService)
	flightPatches := buildServicePatches(nameFlightService, cr.Spec.FlightService)
	gwPatches := buildGatewayPatches(&gw)

	patches := make([]kustypes.Patch, 0, len(restPatches)+len(flightPatches)+len(gwPatches))
	patches = append(patches, restPatches...)
	patches = append(patches, flightPatches...)
	patches = append(patches, gwPatches...)

	resources, err := renderKustomization(basePath, patches, nil)
	if err != nil {
		return fmt.Errorf("rendering manifests: %w", err)
	}

	setDeploymentImage(resources, nameRestService, r.RestImage)
	setDeploymentImage(resources, "kube-rbac-proxy", r.KubeRbacProxyImage)

	setDeploymentImage(resources, nameFlightService, r.FlightImage)

	setConfigMapGlobalNamespace(resources, cr.Namespace)
	setConfigMapFlightServiceAddress(resources)

	audiences := r.resolveTokenReviewAudiences(cr, platCfg)
	if len(audiences) > 0 {
		if !setConfigMapAudiences(resources, audiences) {
			logf.FromContext(ctx).Info("tokenReviewAudiences specified but no config.toml with [auth] section found in rendered manifests")
		}
		setKubeRbacProxyAudiences(resources, audiences)
	}

	annotateDeploymentWithConfigHash(resources, nameFlightService, nameFlightService+"-config")

	return r.applyResources(ctx, cr, cr.Namespace, resources)
}

// ensureInitDataConnectionTypes reads connection type definitions from the
// manifests directory and creates corresponding IDCT CRs in the DCS CR
// namespace so they register under the correct (global) tenant.
func (r *DataConnectServiceReconciler) ensureInitDataConnectionTypes(ctx context.Context, cr *dchv1alpha1.DataConnectService) {
	log := logf.FromContext(ctx)

	typesDir := filepath.Join(r.ManifestsPath, "connection-types")
	entries, err := os.ReadDir(typesDir)
	if err != nil {
		if os.IsNotExist(err) {
			return
		}
		log.Error(err, "failed to read connection-types directory")
		return
	}

	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".yaml" {
			continue
		}

		data, err := os.ReadFile(filepath.Join(typesDir, entry.Name()))
		if err != nil {
			log.Error(err, "failed to read connection type file", "file", entry.Name())
			continue
		}

		var spec connectionTypeFile
		if err := yaml.Unmarshal(data, &spec); err != nil {
			log.Error(err, "failed to parse connection type file", "file", entry.Name())
			continue
		}

		name := strings.TrimSuffix(entry.Name(), ".yaml")
		existing := &dchv1alpha1.InitDataConnectionType{}
		key := types.NamespacedName{Name: name, Namespace: cr.Namespace}
		if err := r.Get(ctx, key, existing); err == nil {
			continue
		}

		idct := &dchv1alpha1.InitDataConnectionType{
			ObjectMeta: metav1.ObjectMeta{
				Name:      name,
				Namespace: cr.Namespace,
			},
			Spec: spec.toIDCTSpec(),
		}
		if err := r.Create(ctx, idct); err != nil {
			if !apierrors.IsAlreadyExists(err) {
				log.Error(err, "failed to create InitDataConnectionType", "name", name)
			}
		} else {
			log.Info("created InitDataConnectionType", "name", name, "namespace", cr.Namespace)
		}
	}
}

type connectionTypeFile struct {
	Name              string                   `json:"name"`
	Provider          string                   `json:"provider"`
	Description       string                   `json:"description"`
	CredentialsFields []connectionTypeFieldDef `json:"credentials_fields"`
}

type connectionTypeFieldDef struct {
	Name         string `json:"name"`
	Label        string `json:"label"`
	Description  string `json:"description"`
	Required     bool   `json:"required"`
	Type         string `json:"type"`
	DefaultValue string `json:"default_value"`
}

func (f *connectionTypeFile) toIDCTSpec() dchv1alpha1.InitDataConnectionTypeSpec {
	fields := make([]dchv1alpha1.CredentialsField, len(f.CredentialsFields))
	for i, cf := range f.CredentialsFields {
		fields[i] = dchv1alpha1.CredentialsField{
			Name:     cf.Name,
			Label:    cf.Label,
			Required: cf.Required,
			Type:     cf.Type,
		}
		if cf.Description != "" {
			fields[i].Description = &cf.Description
		}
		if cf.DefaultValue != "" {
			fields[i].DefaultValue = &cf.DefaultValue
		}
	}
	spec := dchv1alpha1.InitDataConnectionTypeSpec{
		Name:              f.Name,
		Provider:          f.Provider,
		CredentialsFields: fields,
	}
	if f.Description != "" {
		spec.Description = &f.Description
	}
	return spec
}

// deleteInitDataConnectionTypes removes IDCT CRs from the DCS namespace so
// they get re-created on a future install with a fresh database.
func (r *DataConnectServiceReconciler) deleteInitDataConnectionTypes(ctx context.Context, namespace string) {
	log := logf.FromContext(ctx)
	var list dchv1alpha1.InitDataConnectionTypeList
	if err := r.List(ctx, &list, client.InNamespace(namespace)); err != nil {
		log.Error(err, "failed to list InitDataConnectionTypes for cleanup")
		return
	}
	for i := range list.Items {
		if err := r.Delete(ctx, &list.Items[i]); err != nil && !apierrors.IsNotFound(err) {
			log.Error(err, "failed to delete InitDataConnectionType", "name", list.Items[i].Name)
		}
	}
}

// clearSyncedAnnotations removes the dataconnecthub synced annotation from
// connection-type ConfigMaps and connection Secrets so they get re-promoted
// on a future install.
func (r *DataConnectServiceReconciler) clearSyncedAnnotations(ctx context.Context) {
	log := logf.FromContext(ctx)

	var cmList corev1.ConfigMapList
	if err := r.List(ctx, &cmList, client.HasLabels{labelODHConnectionType}); err != nil {
		log.Error(err, "failed to list connection-type ConfigMaps for cleanup")
	} else {
		for i := range cmList.Items {
			cm := &cmList.Items[i]
			if cm.Annotations[annotationDCHSynced] == valueSyncedTrue {
				patch := client.MergeFrom(cm.DeepCopy())
				delete(cm.Annotations, annotationDCHSynced)
				if err := r.Patch(ctx, cm, patch); err != nil {
					log.Error(err, "failed to clear synced annotation", "configmap", cm.Name, "namespace", cm.Namespace)
				}
			}
		}
	}

	var secretList corev1.SecretList
	if err := r.List(ctx, &secretList, client.HasLabels{labelODHDashboard}); err != nil {
		log.Error(err, "failed to list connection Secrets for cleanup")
	} else {
		for i := range secretList.Items {
			s := &secretList.Items[i]
			if s.Annotations[annotationDCHSynced] == valueSyncedTrue {
				patch := client.MergeFrom(s.DeepCopy())
				delete(s.Annotations, annotationDCHSynced)
				if err := r.Patch(ctx, s, patch); err != nil {
					log.Error(err, "failed to clear synced annotation", "secret", s.Name, "namespace", s.Namespace)
				}
			}
		}
	}
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

func (r *DataConnectServiceReconciler) resolveTokenReviewAudiences(cr *dchv1alpha1.DataConnectService, platCfg *platformConfig) []string {
	if cr.Spec.TokenReviewAudiences != nil {
		return cr.Spec.TokenReviewAudiences
	}
	return platCfg.TokenReviewAudiences
}

func (r *DataConnectServiceReconciler) gatewayStatus(ctx context.Context, cr *dchv1alpha1.DataConnectService, platCfg *platformConfig) {
	gw := r.resolveGateway(cr, platCfg)
	cr.Status.HttpRoute = nameDataConnectHub
	cr.Status.Gateway = &dchv1alpha1.Gateway{
		Name:      gw.Name,
		Namespace: gw.Namespace,
	}
	hostname := r.resolveGatewayHostname(ctx, gw.Namespace, gw.Name)
	cr.Status.Addresses = []dchv1alpha1.Addresses{
		{
			Type:  "hostname",
			Value: hostname,
		},
	}
}

// checkGRPCGatewaySupport sets an advisory condition -- and escalates Degraded --
// when the cluster's ingress appears to have HTTP/2 disabled. gRPC (flight-service)
// traffic routed through an OpenShift Route in front of the gateway requires ALPN,
// which OpenShift's router does not negotiate unless HTTP/2 is explicitly enabled.
// Per the ODH PlatformObject contract (Ready must be a true aggregate -- see
// https://github.com/opendatahub-io/odh-platform-utilities/blob/main/docs/platform-object-contract.md),
// this overrides the caller's Ready=True/Phase=Ready when HTTP/2 is confirmed
// disabled, so Ready and Phase never contradict Degraded. It must therefore run
// after the caller's own Ready/Degraded/Phase are set.
func (r *DataConnectServiceReconciler) checkGRPCGatewaySupport(ctx context.Context, cr *dchv1alpha1.DataConnectService) {
	enabled, known := r.http2Enabled(ctx)
	if !known {
		meta.RemoveStatusCondition(&cr.Status.Conditions, conditionTypeGRPCGatewaySupported)
		return
	}
	if enabled {
		r.setCondition(cr, conditionTypeGRPCGatewaySupported, metav1.ConditionTrue, "HTTP2Enabled",
			"Cluster ingress has HTTP/2 enabled; gRPC (flight-service) traffic can negotiate ALPN through the gateway route")
		return
	}
	message := "gRPC (flight-service) traffic routed through an OpenShift Route requires HTTP/2, which OpenShift disables " +
		"by default. A cluster-admin must enable it, e.g.: oc annotate ingresses.config/cluster " +
		http2EnableAnnotation + "=true --overwrite. The Route also needs its own dedicated TLS certificate " +
		"instead of the shared default one for ALPN to negotiate."
	r.setCondition(cr, conditionTypeGRPCGatewaySupported, metav1.ConditionFalse, "HTTP2Disabled", message)
	r.setCondition(cr, conditionTypeDegraded, metav1.ConditionTrue, "GatewayHTTP2Disabled", message)
	r.setCondition(cr, conditionTypeReady, metav1.ConditionFalse, "GatewayHTTP2Disabled", message)
	if cr.Status.Phase == conditionTypeReady {
		cr.Status.Phase = "Not Ready"
	}
}

// http2Enabled reports whether HTTP/2 appears enabled on the cluster's ingress,
// checked cluster-wide (ingresses.config/cluster) and per-IngressController. known
// is false when neither could be read (e.g. non-OpenShift cluster or missing RBAC),
// in which case the caller should not draw a conclusion either way.
func (r *DataConnectServiceReconciler) http2Enabled(ctx context.Context) (enabled, known bool) {
	clusterIngress := &unstructured.Unstructured{}
	clusterIngress.SetGroupVersionKind(schema.GroupVersionKind{
		Group:   "config.openshift.io",
		Version: "v1",
		Kind:    "Ingress",
	})
	if err := r.Get(ctx, types.NamespacedName{Name: "cluster"}, clusterIngress); err == nil {
		known = true
		if clusterIngress.GetAnnotations()[http2EnableAnnotation] == "true" {
			return true, true
		}
	}

	controllers := &unstructured.UnstructuredList{}
	controllers.SetGroupVersionKind(schema.GroupVersionKind{
		Group:   "operator.openshift.io",
		Version: "v1",
		Kind:    "IngressControllerList",
	})
	if err := r.List(ctx, controllers, client.InNamespace("openshift-ingress-operator")); err == nil {
		known = true
		for i := range controllers.Items {
			if controllers.Items[i].GetAnnotations()[http2EnableAnnotation] == "true" {
				return true, true
			}
		}
	}

	return false, known
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

func (r *DataConnectServiceReconciler) platformConfigToReconcile(ctx context.Context, obj client.Object) []reconcile.Request {
	var list dchv1alpha1.DataConnectServiceList
	if err := r.List(ctx, &list); err != nil {
		logf.FromContext(ctx).Error(err, "failed to list DataConnectService CRs for ConfigMap trigger")
		return nil
	}
	requests := make([]reconcile.Request, 0, len(list.Items))
	for i := range list.Items {
		requests = append(requests, reconcile.Request{
			NamespacedName: types.NamespacedName{
				Name:      list.Items[i].Name,
				Namespace: list.Items[i].Namespace,
			},
		})
	}
	return requests
}
