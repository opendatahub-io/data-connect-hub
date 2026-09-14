package tls

import (
	"context"
	cryptotls "crypto/tls"
	"errors"
	"fmt"
	"time"

	configv1 "github.com/openshift/api/config/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/client-go/rest"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

var log = ctrl.Log.WithName("tls")

var cipherSuites = map[string]uint16{
	"TLS_AES_128_GCM_SHA256":               cryptotls.TLS_AES_128_GCM_SHA256,
	"TLS_AES_256_GCM_SHA384":               cryptotls.TLS_AES_256_GCM_SHA384,
	"TLS_CHACHA20_POLY1305_SHA256":         cryptotls.TLS_CHACHA20_POLY1305_SHA256,
	"ECDHE-ECDSA-AES128-GCM-SHA256":        cryptotls.TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
	"ECDHE-RSA-AES128-GCM-SHA256":          cryptotls.TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
	"ECDHE-ECDSA-AES256-GCM-SHA384":        cryptotls.TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
	"ECDHE-RSA-AES256-GCM-SHA384":          cryptotls.TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
	"ECDHE-ECDSA-CHACHA20-POLY1305-SHA256": cryptotls.TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
	"ECDHE-RSA-CHACHA20-POLY1305-SHA256":   cryptotls.TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
	"ECDHE-ECDSA-CHACHA20-POLY1305":        cryptotls.TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
	"ECDHE-RSA-CHACHA20-POLY1305":          cryptotls.TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
}

var tlsVersions = map[configv1.TLSProtocolVersion]uint16{
	"VersionTLS10": cryptotls.VersionTLS10,
	"VersionTLS11": cryptotls.VersionTLS11,
	"VersionTLS12": cryptotls.VersionTLS12,
	"VersionTLS13": cryptotls.VersionTLS13,
}

type Result struct {
	TLSOpts        []func(*cryptotls.Config)
	ProfileSpec    configv1.TLSProfileSpec
	ProfileFetched bool
}

// +kubebuilder:rbac:groups=config.openshift.io,resources=apiservers,verbs=get;list;watch

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
	result := Result{ProfileSpec: intermediateProfile()}
	apiServer := &configv1.APIServer{}
	if err := k8sClient.Get(ctx, client.ObjectKey{Name: "cluster"}, apiServer); err != nil {
		switch {
		case meta.IsNoMatchError(err):
			log.Info("TLS profile unavailable; using hardened defaults")
		case apierrors.IsNotFound(err):
			log.Info("APIServer resource not found; using hardened defaults")
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
	result.ProfileSpec = profileSpec(apiServer.Spec.TLSSecurityProfile)
	return setTLSOpts(result)
}

func intermediateProfile() configv1.TLSProfileSpec {
	return *configv1.TLSProfiles[configv1.TLSProfileIntermediateType]
}

func profileSpec(profile *configv1.TLSSecurityProfile) configv1.TLSProfileSpec {
	if profile == nil {
		return intermediateProfile()
	}

	switch profile.Type {
	case configv1.TLSProfileCustomType:
		if profile.Custom != nil {
			return profile.Custom.TLSProfileSpec
		}
	case configv1.TLSProfileModernType, configv1.TLSProfileOldType:
		return *configv1.TLSProfiles[profile.Type]
	case configv1.TLSProfileIntermediateType, "":
		return intermediateProfile()
	}
	return intermediateProfile()
}

func setTLSOpts(result Result) (Result, error) {
	tlsOpts, err := tlsOpts(result.ProfileSpec)
	if err != nil {
		return Result{}, err
	}
	result.TLSOpts = tlsOpts
	return result, nil
}

func tlsOpts(profile configv1.TLSProfileSpec) ([]func(*cryptotls.Config), error) {
	minVersion, ok := tlsVersions[profile.MinTLSVersion]
	if !ok {
		minVersion = cryptotls.VersionTLS12
	}

	ciphers := make([]uint16, 0, len(profile.Ciphers))
	for _, name := range profile.Ciphers {
		if cipher, ok := cipherSuites[name]; ok {
			ciphers = append(ciphers, cipher)
		} else {
			log.Info("TLS profile cipher unsupported by Go", "cipher", name)
		}
	}
	if len(profile.Ciphers) > 0 && len(ciphers) == 0 {
		return nil, fmt.Errorf("TLS profile has no cipher suites supported by Go")
	}

	return []func(*cryptotls.Config){func(config *cryptotls.Config) {
		config.MinVersion = minVersion
		if len(ciphers) > 0 {
			config.CipherSuites = ciphers
		}
		config.NextProtos = []string{"h2", "http/1.1"}
	}}, nil
}
