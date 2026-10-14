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

package controller

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveControllerNamespace(t *testing.T) {
	origPath := serviceAccountNamespacePath
	t.Cleanup(func() { serviceAccountNamespacePath = origPath })

	saDir := t.TempDir()
	saPath := filepath.Join(saDir, "namespace")

	t.Run("env var takes precedence", func(t *testing.T) {
		t.Setenv(controllerNamespaceEnvVar, "env-ns")
		if err := os.WriteFile(saPath, []byte("file-ns\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		serviceAccountNamespacePath = saPath
		if got := ResolveControllerNamespace(); got != "env-ns" {
			t.Errorf("ResolveControllerNamespace() = %q, want env-ns", got)
		}
	})

	t.Run("falls back to service account namespace", func(t *testing.T) {
		t.Setenv(controllerNamespaceEnvVar, "")
		if err := os.WriteFile(saPath, []byte("pod-ns\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		serviceAccountNamespacePath = saPath
		if got := ResolveControllerNamespace(); got != "pod-ns" {
			t.Errorf("ResolveControllerNamespace() = %q, want pod-ns", got)
		}
	})

	t.Run("empty when namespace cannot be determined", func(t *testing.T) {
		t.Setenv(controllerNamespaceEnvVar, "")
		serviceAccountNamespacePath = filepath.Join(saDir, "does-not-exist")
		if got := ResolveControllerNamespace(); got != "" {
			t.Errorf("ResolveControllerNamespace() = %q, want empty string", got)
		}
	})
}
