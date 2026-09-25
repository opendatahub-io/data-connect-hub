//go:build e2e
// +build e2e

/*
Copyright 2026.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package support

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	cryptorand "crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/pem"
	"fmt"
	"math/big"
	"os/exec"
	"path/filepath"
	"time"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/opendatahub-io/data-connect-hub/dc-controller/test/utils"
)

type testCertificateAuthority struct {
	certificate    *x509.Certificate
	privateKey     *ecdsa.PrivateKey
	certificatePEM []byte
}

func newTestCertificateAuthority() (*testCertificateAuthority, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), cryptorand.Reader)
	if err != nil {
		return nil, fmt.Errorf("generating test CA key: %w", err)
	}
	serial, err := cryptorand.Int(cryptorand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
	if err != nil {
		return nil, fmt.Errorf("generating test CA serial: %w", err)
	}

	now := time.Now()
	template := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: "dc-controller-e2e-ca"},
		NotBefore:             now.Add(-time.Minute),
		NotAfter:              now.Add(24 * time.Hour),
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature,
		BasicConstraintsValid: true,
		IsCA:                  true,
	}
	certificateDER, err := x509.CreateCertificate(cryptorand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		return nil, fmt.Errorf("creating test CA certificate: %w", err)
	}
	certificate, err := x509.ParseCertificate(certificateDER)
	if err != nil {
		return nil, fmt.Errorf("parsing test CA certificate: %w", err)
	}

	return &testCertificateAuthority{
		certificate:    certificate,
		privateKey:     key,
		certificatePEM: pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificateDER}),
	}, nil
}

func (ca *testCertificateAuthority) tlsSecret(name, namespace string, dnsNames ...string) (*corev1.Secret, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), cryptorand.Reader)
	if err != nil {
		return nil, fmt.Errorf("generating TLS key for %s: %w", name, err)
	}
	serial, err := cryptorand.Int(cryptorand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
	if err != nil {
		return nil, fmt.Errorf("generating TLS serial for %s: %w", name, err)
	}

	now := time.Now()
	template := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: dnsNames[0]},
		DNSNames:              dnsNames,
		NotBefore:             now.Add(-time.Minute),
		NotAfter:              now.Add(24 * time.Hour),
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
	}
	certificateDER, err := x509.CreateCertificate(cryptorand.Reader, template, ca.certificate, &key.PublicKey, ca.privateKey)
	if err != nil {
		return nil, fmt.Errorf("creating TLS certificate for %s: %w", name, err)
	}
	keyDER, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		return nil, fmt.Errorf("marshaling TLS key for %s: %w", name, err)
	}

	return &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: namespace},
		Type:       corev1.SecretTypeTLS,
		Data: map[string][]byte{
			"tls.crt": pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificateDER}),
			"tls.key": pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER}),
		},
	}, nil
}

func newTestPassword() (string, error) {
	value := make([]byte, 24)
	if _, err := cryptorand.Read(value); err != nil {
		return "", fmt.Errorf("generating temporary PostgreSQL password: %w", err)
	}
	return hex.EncodeToString(value), nil
}

func installTestPostgres(namespace, password string) error {
	controllerDir, err := utils.GetProjectDir()
	if err != nil {
		return err
	}
	cmd := exec.Command("bash", "hack/install-postgresql.sh",
		"-n", namespace,
		"-r", postgresName,
		"-u", postgresUser,
		"-p", password,
		"-d", postgresDatabase,
		"-t", "180s",
	)
	cmd.Dir = filepath.Dir(controllerDir)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("installing test PostgreSQL: %s: %w", string(output), err)
	}
	return nil
}
