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
	"fmt"

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
	labelODHDashboard           = "opendatahub.io/dashboard"
	annotationConnectionTypeRef = "opendatahub.io/connection-type-ref"
)

// SecretWatcherReconciler watches ODH connection Secrets
// (labelled opendatahub.io/dashboard: "true") and migrates them
// to DCH connections via the REST API. The credential data is
// never stored — only a secret_ref pointing to the original Secret.
// It is a one-shot per Secret: once synced, an annotation is set.
type SecretWatcherReconciler struct {
	client.Client
	Scheme     *runtime.Scheme
	RestClient ConnectionMigrationClient
}

// +kubebuilder:rbac:groups="",resources=secrets,verbs=get;list;watch;update;patch

func (r *SecretWatcherReconciler) Reconcile(ctx context.Context, req ctrl.Request) (ctrl.Result, error) {
	log := logf.FromContext(ctx)

	var secret corev1.Secret
	if err := r.Get(ctx, req.NamespacedName, &secret); err != nil {
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

	if secret.Annotations[annotationDCHSynced] == valueSyncedTrue {
		return ctrl.Result{}, nil
	}

	typeRef := secret.Annotations[annotationConnectionTypeRef]
	if typeRef == "" {
		log.Info("Secret has no connection-type-ref annotation, skipping",
			"name", secret.Name, "namespace", secret.Namespace)
		return ctrl.Result{}, nil
	}

	typeID, err := r.resolveConnectionTypeID(ctx, secret.Namespace, typeRef)
	if err != nil {
		if errors.Is(err, ErrServiceUnavailable) {
			log.Info("REST service unavailable, requeuing", "name", secret.Name)
			return ctrl.Result{RequeueAfter: requeueOnMigrationServiceUnavailable}, nil
		}
		if errors.Is(err, ErrNotFound) {
			log.Info("connection type not yet migrated, requeuing",
				"name", secret.Name, "typeRef", typeRef)
			return ctrl.Result{RequeueAfter: requeueOnMigrationServiceUnavailable}, nil
		}
		log.Error(err, "failed to resolve connection type", "typeRef", typeRef)
		return ctrl.Result{}, nil
	}

	displayName := secret.Annotations[annotationDisplayName]
	if displayName == "" {
		displayName = secret.Name
	}

	conn := Connection{
		Name:                 displayName,
		DataConnectionTypeID: typeID,
		Format:               "tabular",
		CredentialsRef:       &CredentialsRef{Secret: secret.Name},
		Properties:           map[string]string{},
	}

	log.Info("migrating ODH connection", "name", conn.Name, "secretRef", secret.Name, "namespace", secret.Namespace)

	if err := r.RestClient.CreateConnection(ctx, secret.Namespace, conn); err != nil {
		if errors.Is(err, ErrConflict) {
			log.Info("connection already exists, marking synced", "name", conn.Name)
			return r.markSynced(ctx, &secret)
		}
		if errors.Is(err, ErrServiceUnavailable) {
			log.Info("REST service unavailable, requeuing", "name", conn.Name)
			return ctrl.Result{RequeueAfter: requeueOnMigrationServiceUnavailable}, nil
		}
		log.Error(err, "failed to create connection", "name", conn.Name)
		return ctrl.Result{}, nil
	}

	log.Info("connection migrated", "name", conn.Name)
	return r.markSynced(ctx, &secret)
}

func (r *SecretWatcherReconciler) resolveConnectionTypeID(ctx context.Context, namespace, typeRef string) (string, error) {
	types, err := r.RestClient.ListConnectionTypes(ctx, namespace)
	if err != nil {
		return "", err
	}

	for _, t := range types {
		if t.Resource.Name == typeRef {
			return t.Metadata.ID, nil
		}
	}

	return "", fmt.Errorf("%w: connection type %q not found", ErrNotFound, typeRef)
}

func (r *SecretWatcherReconciler) markSynced(ctx context.Context, secret *corev1.Secret) (ctrl.Result, error) {
	patch := client.MergeFrom(secret.DeepCopy())
	if secret.Annotations == nil {
		secret.Annotations = make(map[string]string)
	}
	secret.Annotations[annotationDCHSynced] = valueSyncedTrue
	if err := r.Patch(ctx, secret, patch); err != nil {
		logf.FromContext(ctx).Error(err, "failed to set synced annotation", "name", secret.Name)
		return ctrl.Result{}, err
	}
	return ctrl.Result{}, nil
}

// SetupWithManager sets up the controller with the Manager.
func (r *SecretWatcherReconciler) SetupWithManager(mgr ctrl.Manager) error {
	return ctrl.NewControllerManagedBy(mgr).
		For(&corev1.Secret{}, builder.WithPredicates(
			predicate.NewPredicateFuncs(func(obj client.Object) bool {
				return obj.GetLabels()[labelODHDashboard] == valueSyncedTrue
			}),
		)).
		Named("secretwatcher").
		Complete(r)
}
