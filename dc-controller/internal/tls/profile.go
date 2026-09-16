package tls

import (
	"context"
	cryptotls "crypto/tls"
	"errors"
	"fmt"
	"time"

	configv1 "github.com/openshift/api/config/v1"
	libgocrypto "github.com/openshift/library-go/pkg/crypto"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/client-go/rest"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

var log = ctrl.Log.WithName("tls")

var tlsVersions = map[configv1.TLSProtocolVersion]uint16{
	"VersionTLS10": cryptotls.VersionTLS10,
	"VersionTLS11": cryptotls.VersionTLS11,
	"VersionTLS12": cryptotls.VersionTLS12,
	"VersionTLS13": cryptotls.VersionTLS13,
}

const (
	apiServerName = "cluster"
	http11        = "http/1.1"
)

type Result struct {
	TLSOpts         []func(*cryptotls.Config)
	ProfileSpec     configv1.TLSProfileSpec
	ObservedProfile *configv1.TLSSecurityProfile
	AdherencePolicy configv1.TLSAdherencePolicy
	ProfileFetched  bool
}

func Resolve(ctx context.Context, cfg *rest.Config) (Result, error) {
	scheme := runtime.NewScheme()
	if err := configv1.Install(scheme); err != nil {
		return Result{}, fmt.Errorf("installing OpenShift config scheme: %w", err)
	}

	k8sClient, err := client.New(cfg, client.Options{Scheme: scheme})
	if err != nil {
		return Result{}, fmt.Errorf("creating TLS profile client: %w", err)
	}

	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	return resolve(ctx, k8sClient)
}

func resolve(ctx context.Context, k8sClient client.Reader) (Result, error) {
	result := Result{
		ProfileSpec:     intermediateProfile(),
		AdherencePolicy: configv1.TLSAdherencePolicyNoOpinion,
	}
	apiServer := &configv1.APIServer{}
	if err := k8sClient.Get(ctx, client.ObjectKey{Name: apiServerName}, apiServer); err != nil {
		switch {
		case meta.IsNoMatchError(err):
			log.Info("TLS profile unavailable; using hardened defaults")
		case apierrors.IsNotFound(err):
			log.Info("APIServer resource not found; using hardened defaults")
			result.ProfileFetched = true
		case apierrors.IsServiceUnavailable(err), apierrors.IsTimeout(err), apierrors.IsServerTimeout(err),
			apierrors.IsTooManyRequests(err), errors.Is(err, context.DeadlineExceeded):
			log.Info("Transient error reading TLS profile; using hardened defaults", "error", err)
			result.ProfileFetched = true
		default:
			return Result{}, fmt.Errorf("reading APIServer TLS profile: %w", err)
		}
		return setTLSOpts(result)
	}

	result.ProfileFetched = true
	result.ObservedProfile = cloneProfile(apiServer.Spec.TLSSecurityProfile)
	result.AdherencePolicy = apiServer.Spec.TLSAdherence

	observedSpec, err := profileSpec(apiServer.Spec.TLSSecurityProfile)
	if err == nil {
		result.ProfileSpec = observedSpec
	}
	if err != nil {
		if shouldHonorClusterProfile(result.AdherencePolicy) {
			return Result{}, fmt.Errorf("invalid cluster TLS profile: %w", err)
		}
		log.Info("Invalid cluster TLS profile; using component defaults", "error", err)
		result.ProfileSpec = intermediateProfile()
	}

	configuredResult, err := setTLSOpts(result)
	if err != nil {
		if shouldHonorClusterProfile(result.AdherencePolicy) {
			return Result{}, fmt.Errorf("invalid cluster TLS profile: %w", err)
		}
		log.Info("Unsupported cluster TLS profile settings; using component defaults", "error", err)
		result.ProfileSpec = intermediateProfile()
		return setTLSOpts(result)
	}
	return configuredResult, nil
}

func intermediateProfile() configv1.TLSProfileSpec {
	return *configv1.TLSProfiles[configv1.TLSProfileIntermediateType]
}

func profileSpec(profile *configv1.TLSSecurityProfile) (configv1.TLSProfileSpec, error) {
	if profile == nil {
		return configv1.TLSProfileSpec{}, fmt.Errorf("TLS security profile is missing")
	}

	switch profile.Type {
	case configv1.TLSProfileCustomType:
		if profile.Custom == nil {
			return configv1.TLSProfileSpec{}, fmt.Errorf("custom TLS security profile is missing its Custom settings")
		}
		return profile.Custom.TLSProfileSpec, nil
	case configv1.TLSProfileModernType, configv1.TLSProfileOldType:
		return *configv1.TLSProfiles[profile.Type], nil
	case configv1.TLSProfileIntermediateType:
		return intermediateProfile(), nil
	case "":
		return configv1.TLSProfileSpec{}, fmt.Errorf("TLS security profile type is empty")
	default:
		return configv1.TLSProfileSpec{}, fmt.Errorf("unsupported TLS security profile type %q", profile.Type)
	}
}

func setTLSOpts(result Result) (Result, error) {
	if !shouldHonorClusterProfile(result.AdherencePolicy) {
		result.TLSOpts = legacyTLSOpts()
		return result, nil
	}

	tlsOpts, err := tlsOpts(result.ProfileSpec)
	if err != nil {
		return Result{}, err
	}
	result.TLSOpts = tlsOpts
	return result, nil
}

func legacyTLSOpts() []func(*cryptotls.Config) {
	return []func(*cryptotls.Config){func(config *cryptotls.Config) {
		config.NextProtos = []string{http11}
	}}
}

func tlsOpts(profile configv1.TLSProfileSpec) ([]func(*cryptotls.Config), error) {
	minVersion, ok := tlsVersions[profile.MinTLSVersion]
	if !ok {
		return nil, fmt.Errorf("unsupported minimum TLS version %q", profile.MinTLSVersion)
	}

	ciphers, unsupportedCiphers := cipherCodes(profile.Ciphers)
	for _, name := range unsupportedCiphers {
		log.Info("TLS profile cipher unsupported by Go", "cipher", name)
	}
	if len(profile.Ciphers) > 0 && len(ciphers) == 0 {
		return nil, fmt.Errorf("TLS profile has no cipher suites supported by Go")
	}

	groups, unsupportedGroups := libgocrypto.TLSGroupsToCurveIDs(profile.Groups)
	for _, name := range unsupportedGroups {
		log.Info("TLS profile group unsupported by Go", "group", name)
	}
	if len(profile.Groups) > 0 && len(groups) == 0 {
		return nil, fmt.Errorf("TLS profile has no groups supported by Go")
	}

	return []func(*cryptotls.Config){func(config *cryptotls.Config) {
		config.MinVersion = minVersion
		if minVersion != cryptotls.VersionTLS13 && len(ciphers) > 0 {
			config.CipherSuites = ciphers
		}
		if len(groups) > 0 {
			config.CurvePreferences = groups
		}
		config.NextProtos = []string{http11}
	}}, nil
}

func cipherCodes(names []string) (codes []uint16, unsupported []string) {
	for _, name := range names {
		if code, err := libgocrypto.CipherSuite(name); err == nil {
			codes = append(codes, code)
			continue
		}

		ianaNames := libgocrypto.OpenSSLToIANACipherSuites([]string{name})
		if len(ianaNames) != 1 {
			unsupported = append(unsupported, name)
			continue
		}

		code, err := libgocrypto.CipherSuite(ianaNames[0])
		if err != nil {
			unsupported = append(unsupported, name)
			continue
		}
		codes = append(codes, code)
	}
	return codes, unsupported
}

func shouldHonorClusterProfile(policy configv1.TLSAdherencePolicy) bool {
	switch policy {
	case configv1.TLSAdherencePolicyNoOpinion, configv1.TLSAdherencePolicyLegacyAdheringComponentsOnly:
		return false
	case configv1.TLSAdherencePolicyStrictAllComponents:
		return true
	default:
		log.Info("Unknown TLS adherence policy; treating it as Strict", "policy", policy)
		return true
	}
}

func cloneProfile(profile *configv1.TLSSecurityProfile) *configv1.TLSSecurityProfile {
	if profile == nil {
		return nil
	}
	return profile.DeepCopy()
}
