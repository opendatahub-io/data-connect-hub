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
	"encoding/json"
	"errors"
	"time"

	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/runtime"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/builder"
	"sigs.k8s.io/controller-runtime/pkg/client"
	logf "sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/predicate"
)

const (
	labelODHConnectionType = "opendatahub.io/connection-type"
	annotationDisplayName  = "openshift.io/display-name"
	annotationDescription  = "openshift.io/description"
	annotationDCHSynced    = "dataconnecthub.opendatahub.io/synced"
	valueSyncedTrue        = "true"

	requeueOnMigrationServiceUnavailable = 30 * time.Second
)

// ConfigMapWatcherReconciler watches ODH connection-type ConfigMaps
// (labelled opendatahub.io/connection-type: "true") and promotes them
// to DCH connection types via the REST API. On each reconcile it
// attempts to create the connection type; if it already exists (409)
// the ConfigMap is marked synced and skipped on future reconciles.
type ConfigMapWatcherReconciler struct {
	client.Client
	Scheme     *runtime.Scheme
	RestClient ConnectionTypeClient
}

// +kubebuilder:rbac:groups="",resources=configmaps,verbs=get;list;watch;update;patch

func (r *ConfigMapWatcherReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	var cm corev1.ConfigMap
	if err := r.Get(ctx, req.NamespacedName, &cm); err != nil {
		if apierrors.IsNotFound(err) {
			return ctrl.Result{}, nil
		}
		return ctrl.Result{}, err
	}
	deleting, err := dataConnectServiceDeleting(ctx, r.Client)
	if err != nil {
		return ctrl.Result{}, err
	}
	if deleting {
		return ctrl.Result{}, nil
	}

	alreadySynced := cm.Annotations[annotationDCHSynced] == valueSyncedTrue

	fieldsJSON, ok := cm.Data["fields"]
	if !ok {
		log.Info("ConfigMap has no data.fields key, skipping", "name", cm.Name, "namespace", cm.Namespace)
		return ctrl.Result{}, nil
	}

	ct, err := configMapToConnectionType(&cm, fieldsJSON)
	if err != nil {
		log.Error(err, "failed to parse ConfigMap fields", "name", cm.Name)
		return ctrl.Result{}, nil
	}

	if !alreadySynced {
		log.Info("promoting ODH connection type", "name", ct.Name, "provider", ct.Provider, "namespace", cm.Namespace)
	}

	if err := r.RestClient.CreateConnectionType(ctx, cm.Namespace, ct); err != nil {
		if errors.Is(err, ErrConflict) {
			if !alreadySynced {
				log.Info("connection type already exists, marking synced", "name", ct.Name)
				return r.markSynced(ctx, &cm)
			}
			return ctrl.Result{}, nil
		}
		if errors.Is(err, ErrServiceUnavailable) {
			log.Info("REST service unavailable, requeuing", "name", ct.Name)
			return ctrl.Result{RequeueAfter: requeueOnMigrationServiceUnavailable}, nil
		}
		log.Error(err, "failed to create connection type", "name", ct.Name)
		return ctrl.Result{}, nil
	}

	if alreadySynced {
		log.Info("connection type re-created after database reset", "name", ct.Name)
	} else {
		log.Info("connection type promoted", "name", ct.Name)
	}
	return r.markSynced(ctx, &cm)
}

func (r *ConfigMapWatcherReconciler) markSynced(ctx context.Context, cm *corev1.ConfigMap) (ctrl.Result, error) {
	patch := client.MergeFrom(cm.DeepCopy())
	if cm.Annotations == nil {
		cm.Annotations = make(map[string]string)
	}
	cm.Annotations[annotationDCHSynced] = valueSyncedTrue
	if err := r.Patch(ctx, cm, patch); err != nil {
		logf.FromContext(ctx).Error(err, "failed to set synced annotation", "name", cm.Name)
		return ctrl.Result{}, err
	}
	return ctrl.Result{}, nil
}

// odhField represents a single field entry in an ODH connection-type ConfigMap's data.fields JSON.
type odhField struct {
	Type     string `json:"type"`
	Name     string `json:"name"`
	EnvVar   string `json:"envVar"`
	Required bool   `json:"required"`
}

func configMapToConnectionType(cm *corev1.ConfigMap, fieldsJSON string) (ConnectionType, error) {
	var odhFields []odhField
	if err := json.Unmarshal([]byte(fieldsJSON), &odhFields); err != nil {
		return ConnectionType{}, err
	}

	fields := make([]Field, len(odhFields))
	for i, of := range odhFields {
		fields[i] = Field{
			Name:     of.EnvVar,
			Label:    of.Name,
			Required: of.Required,
			Type:     "string",
		}
	}

	ct := ConnectionType{
		Name:              cm.Name,
		Provider:          cm.Name,
		CredentialsFields: fields,
	}

	if desc, ok := cm.Annotations[annotationDescription]; ok {
		ct.Description = &desc
	}

	return ct, nil
}

// SetupWithManager sets up the controller with the Manager.
func (r *ConfigMapWatcherReconciler) SetupWithManager(mgr ctrl.Manager) error {
	return ctrl.NewControllerManagedBy(mgr).
		For(&corev1.ConfigMap{}, builder.WithPredicates(
			predicate.NewPredicateFuncs(func(obj client.Object) bool {
				return obj.GetLabels()[labelODHConnectionType] == valueSyncedTrue
			}),
		)).
		Named("configmapwatcher").
		Complete(r)
}
