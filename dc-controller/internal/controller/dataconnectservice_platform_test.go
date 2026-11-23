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
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

const platformTestNamespace = "applications"

func TestDataModulePlatformVersionHandshake(t *testing.T) {
	const previousVersion = "3.6.0"
	const targetVersion = "3.7.0"

	for _, tc := range []struct {
		name            string
		previousVersion string
		ready           metav1.ConditionStatus
		wantVersion     string
		wantRequeue     time.Duration
	}{
		{"ready acknowledges upgrade", previousVersion, metav1.ConditionTrue, targetVersion, requeueWhenReady},
		{"final check blocks upgrade", previousVersion, metav1.ConditionFalse, previousVersion, requeueWaitingForReady},
		{"unknown readiness blocks upgrade", previousVersion, metav1.ConditionUnknown, previousVersion, requeueWaitingForReady},
		{"initial install waits for readiness", "", metav1.ConditionFalse, "", requeueWaitingForReady},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx := context.Background()
			scheme := runtime.NewScheme()
			require.NoError(t, dchv1alpha1.AddToScheme(scheme))
			service := &dchv1alpha1.DataConnectService{
				ObjectMeta: metav1.ObjectMeta{Name: testDCSName, Namespace: platformTestNamespace, Generation: 2},
			}
			if tc.previousVersion != "" {
				service.Status.Releases = []dchv1alpha1.ReleaseStatus{{Name: releasePlatform, Version: tc.previousVersion}}
			}
			cli := fake.NewClientBuilder().WithScheme(scheme).
				WithStatusSubresource(service).WithObjects(service).Build()
			r := &DataConnectServiceReconciler{Client: cli}
			key := client.ObjectKeyFromObject(service)
			result, err := r.updateStatus(ctx, reconcile.Request{NamespacedName: key},
				&platformConfig{PlatformVersion: targetVersion}, conditionTypeReady,
				func(cr *dchv1alpha1.DataConnectService) {
					// Simulate the final advisory check overriding nominal readiness.
					r.setCondition(cr, conditionTypeReady, tc.ready, "FinalCheck", "Final readiness check")
					if tc.ready != metav1.ConditionTrue {
						cr.Status.Phase = "Not Ready"
					}
				})
			require.NoError(t, err)
			require.Equal(t, tc.wantRequeue, result.RequeueAfter)
			require.NoError(t, cli.Get(ctx, key, service))
			require.Equal(t, service.Generation, service.Status.ObservedGeneration)
			require.Equal(t, tc.ready == metav1.ConditionTrue,
				meta.IsStatusConditionTrue(service.Status.Conditions, conditionTypeReady))
			var platformRelease *dchv1alpha1.ReleaseStatus
			for i := range service.Status.Releases {
				if service.Status.Releases[i].Name == releasePlatform {
					platformRelease = &service.Status.Releases[i]
				}
			}
			require.NotNil(t, platformRelease)
			require.Equal(t, tc.wantVersion, platformRelease.Version)
		})
	}
}

func TestPlatformConfigOnlyEnqueuesSameNamespace(t *testing.T) {
	ctx := context.Background()
	scheme := runtime.NewScheme()
	require.NoError(t, dchv1alpha1.AddToScheme(scheme))
	cli := fake.NewClientBuilder().WithScheme(scheme).WithObjects(
		&dchv1alpha1.DataConnectService{ObjectMeta: metav1.ObjectMeta{Name: "managed", Namespace: platformTestNamespace}},
		&dchv1alpha1.DataConnectService{ObjectMeta: metav1.ObjectMeta{Name: "standalone", Namespace: "other"}},
	).Build()
	r := &DataConnectServiceReconciler{Client: cli}

	requests := r.platformConfigToReconcile(ctx, &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Name: platformConfigName, Namespace: platformTestNamespace},
	})
	require.Equal(t, []reconcile.Request{{NamespacedName: client.ObjectKey{
		Name: "managed", Namespace: platformTestNamespace,
	}}}, requests)

	requests = r.platformConfigToReconcile(ctx, &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Name: platformConfigName, Namespace: "empty"},
	})
	require.Empty(t, requests)

	// Upstream also uses this mapper for cluster-scoped global-namespace
	// events. An empty event namespace must continue to enqueue every tenant.
	requests = r.platformConfigToReconcile(ctx, &corev1.Namespace{
		ObjectMeta: metav1.ObjectMeta{Name: "global-connections"},
	})
	require.ElementsMatch(t, []reconcile.Request{
		{NamespacedName: client.ObjectKey{Name: "managed", Namespace: platformTestNamespace}},
		{NamespacedName: client.ObjectKey{Name: "standalone", Namespace: "other"}},
	}, requests)
}
