package main

import (
	"testing"
)

func TestValidateFIPSMode(t *testing.T) {
	tests := []struct {
		name     string
		enabled  bool
		enforced bool
		wantErr  string
	}{
		{name: "enabled and enforced", enabled: true, enforced: true},
		{name: "not enabled", wantErr: "FIPS 140-3 mode is not enabled"},
		{name: "not enforced", enabled: true, wantErr: "FIPS 140-3 mode is not enforced"},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			err := validateFIPSMode(test.enabled, test.enforced)
			if test.wantErr == "" {
				if err != nil {
					t.Fatalf("validateFIPSMode() error = %v, want nil", err)
				}
				return
			}
			if err == nil || err.Error() != test.wantErr {
				t.Fatalf("validateFIPSMode() error = %v, want %q", err, test.wantErr)
			}
		})
	}
}
