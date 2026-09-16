package tls

import (
	"context"
	"testing"

	configv1 "github.com/openshift/api/config/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/reconcile"
)

func TestProfileWatcherReconcileDetectsProfileAndAdherenceChanges(t *testing.T) {
	apiServer := &configv1.APIServer{
		ObjectMeta: metav1.ObjectMeta{Name: apiServerName},
		Spec: configv1.APIServerSpec{
			TLSSecurityProfile: &configv1.TLSSecurityProfile{Type: configv1.TLSProfileIntermediateType},
			TLSAdherence:       configv1.TLSAdherencePolicyNoOpinion,
		},
	}

	profileChanges := 0
	adherenceChanges := 0
	watcher := &ProfileWatcher{
		Client:                 newTLSClient(t, apiServer),
		InitialProfile:         cloneProfile(apiServer.Spec.TLSSecurityProfile),
		InitialAdherencePolicy: apiServer.Spec.TLSAdherence,
		OnProfileChange: func(context.Context) {
			profileChanges++
		},
		OnAdherencePolicyChange: func(context.Context) {
			adherenceChanges++
		},
	}
	watcher.lastProfile = watcher.InitialProfile
	watcher.lastAdherencePolicy = watcher.InitialAdherencePolicy

	request := reconcile.Request{NamespacedName: clientObjectKey(apiServerName)}
	if _, err := watcher.Reconcile(context.Background(), request); err != nil {
		t.Fatalf("initial Reconcile() returned an error: %v", err)
	}
	if profileChanges != 0 || adherenceChanges != 0 {
		t.Fatalf("initial reconcile triggered callbacks: profile=%d adherence=%d", profileChanges, adherenceChanges)
	}

	apiServer.Spec.TLSSecurityProfile = &configv1.TLSSecurityProfile{Type: configv1.TLSProfileModernType}
	apiServer.Spec.TLSAdherence = configv1.TLSAdherencePolicyStrictAllComponents
	if err := watcher.Update(context.Background(), apiServer); err != nil {
		t.Fatalf("updating APIServer: %v", err)
	}
	if _, err := watcher.Reconcile(context.Background(), request); err != nil {
		t.Fatalf("profile change Reconcile() returned an error: %v", err)
	}
	if profileChanges != 1 || adherenceChanges != 1 {
		t.Fatalf("profile change callbacks = profile:%d adherence:%d, want 1:1", profileChanges, adherenceChanges)
	}
}

func TestProfileWatcherReconcileRequestsShutdownWhenAPIServerDeleted(t *testing.T) {
	apiServer := &configv1.APIServer{
		ObjectMeta: metav1.ObjectMeta{Name: apiServerName},
		Spec: configv1.APIServerSpec{
			TLSSecurityProfile: &configv1.TLSSecurityProfile{Type: configv1.TLSProfileIntermediateType},
			TLSAdherence:       configv1.TLSAdherencePolicyNoOpinion,
		},
	}

	shutdownRequested := false
	watcher := &ProfileWatcher{
		Client:                 newTLSClient(t, apiServer),
		InitialProfile:         cloneProfile(apiServer.Spec.TLSSecurityProfile),
		InitialAdherencePolicy: apiServer.Spec.TLSAdherence,
		OnProfileChange: func(context.Context) {
			shutdownRequested = true
		},
	}
	watcher.lastProfile = watcher.InitialProfile
	watcher.lastAdherencePolicy = watcher.InitialAdherencePolicy

	if err := watcher.Delete(context.Background(), apiServer); err != nil {
		t.Fatalf("deleting APIServer: %v", err)
	}

	request := reconcile.Request{NamespacedName: clientObjectKey(apiServerName)}
	if _, err := watcher.Reconcile(context.Background(), request); err != nil {
		t.Fatalf("deleted APIServer Reconcile() returned an error: %v", err)
	}
	if !shutdownRequested {
		t.Fatal("deleted APIServer did not request shutdown")
	}
}

func TestProfileWatcherDetectsMalformedProfileRepair(t *testing.T) {
	apiServer := &configv1.APIServer{
		ObjectMeta: metav1.ObjectMeta{Name: apiServerName},
		Spec: configv1.APIServerSpec{
			TLSSecurityProfile: &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType},
			TLSAdherence:       configv1.TLSAdherencePolicyLegacyAdheringComponentsOnly,
		},
	}

	profileChanges := 0
	watcher := &ProfileWatcher{
		Client:                 newTLSClient(t, apiServer),
		InitialProfile:         cloneProfile(apiServer.Spec.TLSSecurityProfile),
		InitialAdherencePolicy: apiServer.Spec.TLSAdherence,
		OnProfileChange: func(context.Context) {
			profileChanges++
		},
	}
	watcher.lastProfile = cloneProfile(watcher.InitialProfile)
	watcher.lastAdherencePolicy = watcher.InitialAdherencePolicy

	request := reconcile.Request{NamespacedName: clientObjectKey(apiServerName)}
	if _, err := watcher.Reconcile(context.Background(), request); err != nil {
		t.Fatalf("initial malformed profile Reconcile() returned an error: %v", err)
	}

	apiServer.Spec.TLSSecurityProfile.Custom = &configv1.CustomTLSProfile{TLSProfileSpec: configv1.TLSProfileSpec{
		MinTLSVersion: configv1.VersionTLS12,
	}}
	if err := watcher.Update(context.Background(), apiServer); err != nil {
		t.Fatalf("updating repaired APIServer: %v", err)
	}
	if _, err := watcher.Reconcile(context.Background(), request); err != nil {
		t.Fatalf("repaired profile Reconcile() returned an error: %v", err)
	}
	if profileChanges != 1 {
		t.Fatalf("profile changes = %d, want 1 after malformed profile repair", profileChanges)
	}
}

func clientObjectKey(name string) client.ObjectKey {
	return client.ObjectKey{Name: name}
}
