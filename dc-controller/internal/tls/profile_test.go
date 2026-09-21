package tls

import (
	"context"
	cryptotls "crypto/tls"
	"reflect"
	"testing"

	configv1 "github.com/openshift/api/config/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
)

func TestResolveAppliesTLSProfileAccordingToAdherence(t *testing.T) {
	tests := []struct {
		name           string
		adherence      configv1.TLSAdherencePolicy
		wantMinVersion uint16
	}{
		{
			name:           "no opinion preserves legacy defaults",
			adherence:      configv1.TLSAdherencePolicyNoOpinion,
			wantMinVersion: 0,
		},
		{
			name:           "legacy components only preserves legacy defaults",
			adherence:      configv1.TLSAdherencePolicyLegacyAdheringComponentsOnly,
			wantMinVersion: 0,
		},
		{
			name:           "strict adherence uses the cluster profile",
			adherence:      configv1.TLSAdherencePolicyStrictAllComponents,
			wantMinVersion: cryptotls.VersionTLS10,
		},
		{
			name:           "unknown adherence is strict",
			adherence:      configv1.TLSAdherencePolicy("FuturePolicy"),
			wantMinVersion: cryptotls.VersionTLS10,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			apiServer := &configv1.APIServer{
				ObjectMeta: metav1.ObjectMeta{Name: apiServerName},
				Spec: configv1.APIServerSpec{
					TLSSecurityProfile: &configv1.TLSSecurityProfile{Type: configv1.TLSProfileOldType},
					TLSAdherence:       test.adherence,
				},
			}

			result, err := resolve(context.Background(), newTLSClient(t, apiServer))
			if err != nil {
				t.Fatalf("resolve() returned an error: %v", err)
			}
			if !reflect.DeepEqual(result.ProfileSpec, *configv1.TLSProfiles[configv1.TLSProfileOldType]) {
				t.Fatalf("ProfileSpec = %#v, want the observed Old profile", result.ProfileSpec)
			}
			if result.AdherencePolicy != test.adherence {
				t.Fatalf("AdherencePolicy = %q, want %q", result.AdherencePolicy, test.adherence)
			}

			config := &cryptotls.Config{}
			for _, option := range result.TLSOpts {
				option(config)
			}
			if config.MinVersion != test.wantMinVersion {
				t.Fatalf("MinVersion = %d, want %d", config.MinVersion, test.wantMinVersion)
			}
			if !reflect.DeepEqual(config.NextProtos, []string{http11}) {
				t.Fatalf("NextProtos = %v, want [http/1.1]", config.NextProtos)
			}
		})
	}
}

func TestResolveRegistersWatcherWhenAPIServerIsNotFound(t *testing.T) {
	result, err := resolve(context.Background(), newTLSClient(t))
	if err != nil {
		t.Fatalf("resolve() returned an error: %v", err)
	}
	if !result.ProfileFetched {
		t.Fatal("ProfileFetched = false, want true so the watcher can observe APIServer creation")
	}
}

func TestTLSOptsAppliesConfiguredGroups(t *testing.T) {
	profile := configv1.TLSProfileSpec{
		MinTLSVersion: configv1.VersionTLS12,
		Groups: []configv1.TLSGroup{
			configv1.TLSGroupX25519MLKEM768,
			configv1.TLSGroupSecP256r1,
		},
	}

	options, err := tlsOpts(profile)
	if err != nil {
		t.Fatalf("tlsOpts() returned an error: %v", err)
	}

	config := &cryptotls.Config{}
	for _, option := range options {
		option(config)
	}

	want := []cryptotls.CurveID{cryptotls.X25519MLKEM768, cryptotls.CurveP256}
	if !reflect.DeepEqual(config.CurvePreferences, want) {
		t.Fatalf("CurvePreferences = %v, want %v", config.CurvePreferences, want)
	}
}

func TestTLSOptsMapsGoSupportedOldProfileCiphers(t *testing.T) {
	options, err := tlsOpts(*configv1.TLSProfiles[configv1.TLSProfileOldType])
	if err != nil {
		t.Fatalf("tlsOpts() returned an error: %v", err)
	}

	config := &cryptotls.Config{}
	for _, option := range options {
		option(config)
	}

	want := []uint16{
		cryptotls.TLS_AES_128_GCM_SHA256,
		cryptotls.TLS_AES_256_GCM_SHA384,
		cryptotls.TLS_CHACHA20_POLY1305_SHA256,
		cryptotls.TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
		cryptotls.TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
		cryptotls.TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
		cryptotls.TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
		cryptotls.TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
		cryptotls.TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
		cryptotls.TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA256,
		cryptotls.TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256,
		cryptotls.TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA,
		cryptotls.TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA,
		cryptotls.TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA,
		cryptotls.TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA,
		cryptotls.TLS_RSA_WITH_AES_128_GCM_SHA256,
		cryptotls.TLS_RSA_WITH_AES_256_GCM_SHA384,
		cryptotls.TLS_RSA_WITH_AES_128_CBC_SHA256,
		cryptotls.TLS_RSA_WITH_AES_128_CBC_SHA,
		cryptotls.TLS_RSA_WITH_AES_256_CBC_SHA,
		cryptotls.TLS_RSA_WITH_3DES_EDE_CBC_SHA,
	}
	if !reflect.DeepEqual(config.CipherSuites, want) {
		t.Fatalf("CipherSuites = %v, want %v", config.CipherSuites, want)
	}
}

func TestTLSOptsLeavesCipherSuitesUnsetForTLS13(t *testing.T) {
	profile := configv1.TLSProfileSpec{
		Ciphers:       []string{"TLS_AES_128_GCM_SHA256"},
		MinTLSVersion: configv1.VersionTLS13,
	}

	options, err := tlsOpts(profile)
	if err != nil {
		t.Fatalf("tlsOpts() returned an error: %v", err)
	}

	config := &cryptotls.Config{}
	for _, option := range options {
		option(config)
	}
	if config.CipherSuites != nil {
		t.Fatalf("CipherSuites = %v, want nil for TLS 1.3", config.CipherSuites)
	}
}

func TestTLSOptsUsesHTTP11Only(t *testing.T) {
	options, err := tlsOpts(intermediateProfile())
	if err != nil {
		t.Fatalf("tlsOpts() returned an error: %v", err)
	}

	config := &cryptotls.Config{}
	for _, option := range options {
		option(config)
	}

	want := []string{"http/1.1"}
	if !reflect.DeepEqual(config.NextProtos, want) {
		t.Fatalf("NextProtos = %v, want %v", config.NextProtos, want)
	}
}

func TestResolveRejectsMalformedStrictProfiles(t *testing.T) {
	tests := []struct {
		name    string
		profile *configv1.TLSSecurityProfile
	}{
		{
			name:    "missing custom settings",
			profile: &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType},
		},
		{
			name: "empty minimum version",
			profile: &configv1.TLSSecurityProfile{
				Type:   configv1.TLSProfileCustomType,
				Custom: &configv1.CustomTLSProfile{TLSProfileSpec: configv1.TLSProfileSpec{}},
			},
		},
		{
			name: "unknown minimum version",
			profile: &configv1.TLSSecurityProfile{
				Type: configv1.TLSProfileCustomType,
				Custom: &configv1.CustomTLSProfile{TLSProfileSpec: configv1.TLSProfileSpec{
					MinTLSVersion: configv1.TLSProtocolVersion("VersionTLS99"),
				}},
			},
		},
		{
			name:    "unknown profile type",
			profile: &configv1.TLSSecurityProfile{Type: configv1.TLSProfileType("FutureProfile")},
		},
		{
			name:    "missing profile",
			profile: nil,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			apiServer := &configv1.APIServer{
				ObjectMeta: metav1.ObjectMeta{Name: apiServerName},
				Spec: configv1.APIServerSpec{
					TLSSecurityProfile: test.profile,
					TLSAdherence:       configv1.TLSAdherencePolicyStrictAllComponents,
				},
			}

			if _, err := resolve(context.Background(), newTLSClient(t, apiServer)); err == nil {
				t.Fatal("resolve() returned nil error for malformed Strict profile")
			}
		})
	}
}

func TestResolveUsesFallbackForMalformedLegacyProfile(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType}
	apiServer := &configv1.APIServer{
		ObjectMeta: metav1.ObjectMeta{Name: apiServerName},
		Spec: configv1.APIServerSpec{
			TLSSecurityProfile: profile,
			TLSAdherence:       configv1.TLSAdherencePolicyLegacyAdheringComponentsOnly,
		},
	}

	result, err := resolve(context.Background(), newTLSClient(t, apiServer))
	if err != nil {
		t.Fatalf("resolve() returned an error: %v", err)
	}
	if result.ObservedProfile == nil || result.ObservedProfile.Custom != nil {
		t.Fatalf("ObservedProfile = %#v, want the malformed Custom profile preserved", result.ObservedProfile)
	}

	config := &cryptotls.Config{}
	for _, option := range result.TLSOpts {
		option(config)
	}
	if config.MinVersion != 0 {
		t.Fatalf("MinVersion = %d, want the legacy default 0", config.MinVersion)
	}
}

func newTLSClient(t *testing.T, objects ...client.Object) client.Client {
	t.Helper()
	scheme := runtime.NewScheme()
	if err := configv1.Install(scheme); err != nil {
		t.Fatalf("installing OpenShift config scheme: %v", err)
	}
	return fake.NewClientBuilder().WithScheme(scheme).WithObjects(objects...).Build()
}
