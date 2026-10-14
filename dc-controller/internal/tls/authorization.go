package tls

import (
	"context"
	"time"

	configv1 "github.com/openshift/api/config/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

const authorizationCheckInterval = 30 * time.Second

func WatchAPIServerReadAuthorization(ctx context.Context, reader client.Reader, cancel context.CancelFunc) error {
	ticker := time.NewTicker(authorizationCheckInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
			checkAPIServerReadAuthorization(ctx, reader, cancel)
		}
	}
}

func checkAPIServerReadAuthorization(ctx context.Context, reader client.Reader, cancel context.CancelFunc) {
	err := reader.Get(ctx, client.ObjectKey{Name: apiServerName}, &configv1.APIServer{})
	if apierrors.IsForbidden(err) || apierrors.IsUnauthorized(err) {
		log.Error(err, "Lost authorization to read the TLS policy; stopping serving")
		cancel()
	} else if err != nil {
		log.V(1).Info("Could not check TLS policy read authorization", "error", err)
	}
}
