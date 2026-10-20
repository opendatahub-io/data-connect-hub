package tls

import (
	"context"
	"errors"
	"testing"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/runtime/schema"
)

const apiServerResource = "apiservers"

func TestCheckAPIServerReadAuthorization(t *testing.T) {
	for _, tt := range []struct {
		name       string
		reader     failingTLSReader
		wantCancel bool
	}{
		{
			name:       "forbidden",
			reader:     failingTLSReader{err: apierrors.NewForbidden(schema.GroupResource{Resource: apiServerResource}, apiServerName, errors.New("denied"))},
			wantCancel: true,
		},
		{
			name:       "unauthorized",
			reader:     failingTLSReader{err: apierrors.NewUnauthorized("denied")},
			wantCancel: true,
		},
		{name: "transient error", reader: failingTLSReader{err: errors.New("temporary failure")}},
		{name: "successful read", reader: failingTLSReader{}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			called := false
			checkAPIServerReadAuthorization(context.Background(), tt.reader, func() { called = true })
			if called != tt.wantCancel {
				t.Fatalf("cancel called = %t, want %t", called, tt.wantCancel)
			}
		})
	}
}
