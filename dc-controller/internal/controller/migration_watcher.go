package controller

import (
	"context"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

func dataConnectServiceDeleting(ctx context.Context, reader client.Reader) (bool, error) {
	var list dchv1alpha1.DataConnectServiceList
	if err := reader.List(ctx, &list); err != nil {
		if apierrors.IsNotFound(err) {
			return false, nil
		}
		return false, err
	}

	for i := range list.Items {
		if !list.Items[i].DeletionTimestamp.IsZero() {
			return true, nil
		}
	}
	return false, nil
}
