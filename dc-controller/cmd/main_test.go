package main

import (
	"testing"
)

func TestValidateFIPSRequest(t *testing.T) {
	if err := validateFIPSRequest("1"); err != nil {
		t.Fatalf("validateFIPSRequest(\"1\") error = %v, want nil", err)
	}
	err := validateFIPSRequest("")
	if err == nil || err.Error() != "GOLANG_FIPS=1 is not set" {
		t.Fatalf("validateFIPSRequest(\"\") error = %v, want GOLANG_FIPS=1 is not set", err)
	}
}
