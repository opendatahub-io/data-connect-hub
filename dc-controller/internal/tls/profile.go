package tls

import (
	"context"
	cryptotls "crypto/tls"
	"fmt"
	"strings"
	"time"

	platformtls "github.com/opendatahub-io/odh-platform-utilities/framework/tls"
	configv1 "github.com/openshift/api/config/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/client-go/rest"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

var log = ctrl.Log.WithName("tls")

const (
	apiServerName = "cluster"
	http11        = "http/1.1"
)

type Result struct {
	Spec            configv1.TLSProfileSpec
	AdherencePolicy configv1.TLSAdherencePolicy
	UsedFallback    bool
	Watchable       bool
	TLSOpts         []func(*cryptotls.Config)
}

func Resolve(ctx context.Context, cfg *rest.Config) (Result, error) {
	scheme := runtime.NewScheme()
	if err := configv1.Install(scheme); err != nil {
		return Result{}, fmt.Errorf("installing OpenShift config scheme: %w", err)
	}

	bootstrapConfig := rest.CopyConfig(cfg)
	bootstrapConfig.Timeout = 10 * time.Second
	k8sClient, err := client.New(bootstrapConfig, client.Options{Scheme: scheme})
	if err != nil {
		return Result{}, fmt.Errorf("creating TLS profile client: %w", err)
	}

	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	return resolveProfile(ctx, k8sClient)
}

func resolveProfile(ctx context.Context, reader client.Reader) (Result, error) {
	loaded, err := platformtls.LoadWithAdherence(ctx, reader)
	if err != nil {
		return Result{}, fmt.Errorf("resolving cluster TLS profile: %w", err)
	}

	result := Result{
		Spec:            loaded.Spec,
		AdherencePolicy: loaded.AdherencePolicy,
		UsedFallback:    loaded.UsedFallback,
		Watchable:       loaded.Watchable,
	}
	if loaded.UsedFallback {
		log.Info("APIServer TLS profile unavailable; using Intermediate fallback")
	}
	tlsOption, unsupportedCiphers, err := platformtls.ConfigFromProfileWithError(loaded.Spec)
	if err != nil {
		return Result{}, fmt.Errorf("building TLS options from profile: %w", err)
	}
	for _, cipher := range unsupportedCiphers {
		log.Info("TLS profile cipher unsupported by Go", "cipher", cipher)
	}

	var usable []uint16
	if loaded.Spec.MinTLSVersion != configv1.VersionTLS13 {
		scratch := &cryptotls.Config{}
		tlsOption(scratch)
		usable = usableCipherSuites(scratch.CipherSuites)
		if len(loaded.Spec.Ciphers) > 0 && len(usable) == 0 {
			return Result{}, fmt.Errorf("failed to find usable cipher suites for TLS <= 1.2: all %d ciphers in TLS profile are TLS 1.3-only or unimplemented by Go", len(loaded.Spec.Ciphers))
		}
	}

	result.TLSOpts = []func(*cryptotls.Config){func(config *cryptotls.Config) {
		tlsOption(config)
		if len(loaded.Spec.Ciphers) > 0 && loaded.Spec.MinTLSVersion != configv1.VersionTLS13 {
			config.CipherSuites = usable
		}
		config.NextProtos = []string{http11}
	}}
	return result, nil
}

func usableCipherSuites(mapped []uint16) []uint16 {
	usable := make(map[uint16]struct{})
	for _, suite := range cryptotls.CipherSuites() {
		usable[suite.ID] = struct{}{}
	}
	for _, suite := range cryptotls.InsecureCipherSuites() {
		if !strings.Contains(suite.Name, "RC4") {
			usable[suite.ID] = struct{}{}
		}
	}

	filtered := make([]uint16, 0, len(mapped))
	for _, id := range mapped {
		if id >= 0x1300 && id <= 0x13ff {
			continue
		}
		if _, ok := usable[id]; ok {
			filtered = append(filtered, id)
		}
	}
	return filtered
}
