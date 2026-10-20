package tls

import (
	"context"
	cryptotls "crypto/tls"
	"errors"
	"reflect"
	"strings"
	"testing"

	configv1 "github.com/openshift/api/config/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
)

const usableECDHECipher = "ECDHE-ECDSA-AES128-GCM-SHA256"

func TestResolveRejectsEmptyCiphersUnderStrictAndUnknownAdherence(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{MinTLSVersion: configv1.VersionTLS12},
	}}
	for _, policy := range []configv1.TLSAdherencePolicy{
		configv1.TLSAdherencePolicyStrictAllComponents,
		"FuturePolicy",
	} {
		t.Run(string(policy), func(t *testing.T) {
			_, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, policy)))
			if err == nil {
				t.Fatal("resolveProfile() returned nil error for an empty cipher list below TLS 1.3")
			}
			if !strings.Contains(err.Error(), "no cipher suites") {
				t.Fatalf("error = %v, want a no-cipher-suites rejection", err)
			}
		})
	}
}

func TestResolveModernProfileKeepsGoManagedTLS13Ciphers(t *testing.T) {
	apiServer := tlsAPIServer(&configv1.TLSSecurityProfile{Type: configv1.TLSProfileModernType}, configv1.TLSAdherencePolicyStrictAllComponents)
	result, err := resolveProfile(context.Background(), newTLSClient(t, apiServer))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error: %v", err)
	}
	config := applyTLSOpts(result.TLSOpts)
	if config.MinVersion != cryptotls.VersionTLS13 {
		t.Fatalf("MinVersion = %d, want TLS 1.3", config.MinVersion)
	}
	if len(config.CipherSuites) != 0 {
		t.Fatalf("CipherSuites = %v, want Go-managed TLS 1.3 cipher selection", config.CipherSuites)
	}
}

func TestResolveNilProfileOnSuccessfulReadUsesIntermediate(t *testing.T) {
	apiServer := tlsAPIServer(nil, configv1.TLSAdherencePolicyStrictAllComponents)
	result, err := resolveProfile(context.Background(), newTLSClient(t, apiServer))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error: %v", err)
	}
	if !reflect.DeepEqual(result.Spec, *configv1.TLSProfiles[configv1.TLSProfileIntermediateType]) {
		t.Fatalf("Spec = %#v, want Intermediate for a nil profile on a successful read", result.Spec)
	}
	if config := applyTLSOpts(result.TLSOpts); config.MinVersion != cryptotls.VersionTLS12 {
		t.Fatalf("MinVersion = %d, want TLS 1.2", config.MinVersion)
	}
}

func TestResolveStrictProfileProducesProfileTLSOptions(t *testing.T) {
	apiServer := tlsAPIServer(&configv1.TLSSecurityProfile{Type: configv1.TLSProfileOldType}, configv1.TLSAdherencePolicyStrictAllComponents)
	result, err := resolveProfile(context.Background(), newTLSClient(t, apiServer))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error: %v", err)
	}
	if !reflect.DeepEqual(result.Spec, *configv1.TLSProfiles[configv1.TLSProfileOldType]) {
		t.Fatalf("Spec = %#v, want observed Old profile", result.Spec)
	}
	config := applyTLSOpts(result.TLSOpts)
	if config.MinVersion != cryptotls.VersionTLS10 {
		t.Fatalf("MinVersion = %d, want TLS 1.0", config.MinVersion)
	}
	if !reflect.DeepEqual(config.NextProtos, []string{http11}) {
		t.Fatalf("NextProtos = %v, want [http/1.1]", config.NextProtos)
	}
}

func TestResolveHonorsStrictCustomMinimumTLSVersion(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{
			MinTLSVersion: configv1.VersionTLS11,
			Ciphers:       []string{"ECDHE-RSA-AES128-GCM-SHA256"},
		},
	}}
	result, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, configv1.TLSAdherencePolicyStrictAllComponents)))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error: %v", err)
	}
	if config := applyTLSOpts(result.TLSOpts); config.MinVersion != cryptotls.VersionTLS11 {
		t.Fatalf("MinVersion = %d, want TLS 1.1 from custom profile", config.MinVersion)
	}
}

func TestResolveRejectsInvalidStrictProfile(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{MinTLSVersion: "VersionTLS99"},
	}}
	_, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, configv1.TLSAdherencePolicyStrictAllComponents)))
	if err == nil {
		t.Fatal("resolveProfile() returned nil error for an invalid strict profile")
	}
}

func TestResolveFallbacksWhenConfigAPIIsMissing(t *testing.T) {
	err := &meta.NoResourceMatchError{PartialResource: schema.GroupVersionResource{
		Group: "config.openshift.io", Version: "v1", Resource: apiServerResource,
	}}
	result, resolveErr := resolveProfile(context.Background(), failingTLSReader{err: err})
	if resolveErr != nil {
		t.Fatalf("resolveProfile() returned an error: %v", resolveErr)
	}
	assertFallback(t, result, false)
}

func TestResolveFallbacksAndWatchesAfterTransientReadError(t *testing.T) {
	for _, readErr := range []error{
		apierrors.NewServiceUnavailable("unavailable"),
		apierrors.NewTimeoutError("timed out", 0),
		context.DeadlineExceeded,
	} {
		t.Run(readErr.Error(), func(t *testing.T) {
			result, err := resolveProfile(context.Background(), failingTLSReader{err: readErr})
			if err != nil {
				t.Fatalf("resolveProfile() returned an error: %v", err)
			}
			assertFallback(t, result, true)
		})
	}
}

func TestResolveReturnsDeniedReadErrors(t *testing.T) {
	for _, err := range []error{
		apierrors.NewForbidden(schema.GroupResource{Group: "config.openshift.io", Resource: "apiservers"}, "cluster", errors.New("denied")),
		apierrors.NewUnauthorized("denied"),
	} {
		t.Run(err.Error(), func(t *testing.T) {
			if _, gotErr := resolveProfile(context.Background(), failingTLSReader{err: err}); gotErr == nil {
				t.Fatal("resolveProfile() returned nil error for a denied read")
			}
		})
	}
}

func TestResolveAppliesProfileUnderLenientPolicies(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{
			MinTLSVersion: configv1.VersionTLS12,
			Ciphers:       []string{"ECDHE-RSA-AES128-GCM-SHA256"},
		},
	}}
	for _, policy := range []configv1.TLSAdherencePolicy{
		configv1.TLSAdherencePolicyNoOpinion,
		configv1.TLSAdherencePolicyLegacyAdheringComponentsOnly,
	} {
		t.Run(string(policy), func(t *testing.T) {
			result, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, policy)))
			if err != nil {
				t.Fatalf("resolveProfile() returned an error: %v", err)
			}
			config := applyTLSOpts(result.TLSOpts)
			if config.MinVersion != cryptotls.VersionTLS12 {
				t.Fatalf("MinVersion = %d, want TLS 1.2 from cluster profile", config.MinVersion)
			}
			if !reflect.DeepEqual(config.CipherSuites, []uint16{cryptotls.TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256}) {
				t.Fatalf("CipherSuites = %v, want cluster profile cipher", config.CipherSuites)
			}
			if !reflect.DeepEqual(config.NextProtos, []string{http11}) {
				t.Fatalf("NextProtos = %v, want [http/1.1]", config.NextProtos)
			}
		})
	}
}

func TestResolveRejectsAllUnusableCiphersUnderNoOpinion(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{
			MinTLSVersion: configv1.VersionTLS12,
			Ciphers:       []string{"TLS_AES_128_GCM_SHA256"},
		},
	}}
	_, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, configv1.TLSAdherencePolicyNoOpinion)))
	if err == nil {
		t.Fatal("resolveProfile() returned nil error for a TLS 1.3-only cipher under TLS 1.2")
	}
}

func TestResolveAcceptsEmptyCiphersUnderNoOpinion(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{MinTLSVersion: configv1.VersionTLS12},
	}}
	result, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, configv1.TLSAdherencePolicyNoOpinion)))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error for an empty NoOpinion cipher list: %v", err)
	}
	config := applyTLSOpts(result.TLSOpts)
	if config.MinVersion != cryptotls.VersionTLS12 || len(config.CipherSuites) != 0 {
		t.Fatalf("TLS config = {MinVersion: %d, CipherSuites: %v}, want TLS 1.2 and Go-default ciphers", config.MinVersion, config.CipherSuites)
	}
}

func TestResolveKeepsSupportedCiphersWhenSomeAreUnsupported(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{
			MinTLSVersion: configv1.VersionTLS12,
			Ciphers:       []string{usableECDHECipher, "not-a-real-cipher"},
		},
	}}
	result, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, configv1.TLSAdherencePolicyStrictAllComponents)))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error: %v", err)
	}
	config := applyTLSOpts(result.TLSOpts)
	if config.MinVersion != cryptotls.VersionTLS12 {
		t.Fatalf("MinVersion = %d, want TLS 1.2", config.MinVersion)
	}
	if len(config.CipherSuites) != 1 || config.CipherSuites[0] != cryptotls.TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 {
		t.Fatalf("CipherSuites = %v, want supported ECDHE-ECDSA AES-128-GCM suite", config.CipherSuites)
	}
}

func TestResolveDropsRC4Ciphers(t *testing.T) {
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{
			MinTLSVersion: configv1.VersionTLS12,
			Ciphers:       []string{"RC4-SHA", "TLS_RSA_WITH_RC4_128_SHA", usableECDHECipher},
		},
	}}
	result, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, configv1.TLSAdherencePolicyStrictAllComponents)))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error: %v", err)
	}
	config := applyTLSOpts(result.TLSOpts)
	if len(config.CipherSuites) != 1 || config.CipherSuites[0] != cryptotls.TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 {
		t.Fatalf("CipherSuites = %v, want the RC4 entries dropped and the usable suite kept", config.CipherSuites)
	}

	rc4Only := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{
			MinTLSVersion: configv1.VersionTLS12,
			Ciphers:       []string{"TLS_RSA_WITH_RC4_128_SHA"},
		},
	}}
	if _, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(rc4Only, configv1.TLSAdherencePolicyStrictAllComponents))); err == nil {
		t.Fatal("resolveProfile() returned nil error for an all-RC4 cipher list")
	}
}

func TestResolveDropsUnnameableInsecureCiphers(t *testing.T) {
	// The ECDHE RSA 3DES suite is implemented by Go but no OpenShift profile
	// can name it (Old carries only the RSA key-exchange 3DES variant), so the
	// runtime cipher vocabulary rejects it.
	profile := &configv1.TLSSecurityProfile{Type: configv1.TLSProfileCustomType, Custom: &configv1.CustomTLSProfile{
		TLSProfileSpec: configv1.TLSProfileSpec{
			MinTLSVersion: configv1.VersionTLS12,
			Ciphers:       []string{"ECDHE-RSA-DES-CBC3-SHA", usableECDHECipher},
		},
	}}
	result, err := resolveProfile(context.Background(), newTLSClient(t, tlsAPIServer(profile, configv1.TLSAdherencePolicyStrictAllComponents)))
	if err != nil {
		t.Fatalf("resolveProfile() returned an error: %v", err)
	}
	config := applyTLSOpts(result.TLSOpts)
	if len(config.CipherSuites) != 1 || config.CipherSuites[0] != cryptotls.TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 {
		t.Fatalf("CipherSuites = %v, want the unnameable 3DES entry dropped and the usable suite kept", config.CipherSuites)
	}
}

type failingTLSReader struct {
	client.Reader
	err error
}

func (r failingTLSReader) Get(context.Context, client.ObjectKey, client.Object, ...client.GetOption) error {
	return r.err
}

func tlsAPIServer(profile *configv1.TLSSecurityProfile, policy configv1.TLSAdherencePolicy) *configv1.APIServer {
	return &configv1.APIServer{
		ObjectMeta: metav1.ObjectMeta{Name: apiServerName},
		Spec:       configv1.APIServerSpec{TLSSecurityProfile: profile, TLSAdherence: policy},
	}
}

func applyTLSOpts(options []func(*cryptotls.Config)) *cryptotls.Config {
	config := &cryptotls.Config{}
	for _, option := range options {
		option(config)
	}
	return config
}

func assertFallback(t *testing.T, result Result, watchable bool) {
	t.Helper()
	if !reflect.DeepEqual(result.Spec, *configv1.TLSProfiles[configv1.TLSProfileIntermediateType]) {
		t.Fatalf("Spec = %#v, want Intermediate fallback", result.Spec)
	}
	if !result.UsedFallback {
		t.Fatal("UsedFallback = false, want true")
	}
	if result.Watchable != watchable {
		t.Fatalf("Watchable = %t, want %t", result.Watchable, watchable)
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
