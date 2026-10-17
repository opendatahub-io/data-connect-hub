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
	"strings"
)

// controllerNamespaceEnvVar optionally overrides the namespace used to scope
// the NetworkPolicies the controller renders for the REST and flight services.
const controllerNamespaceEnvVar = "CONTROLLER_NAMESPACE"

// serviceAccountNamespacePath is the projected service-account volume path
// holding the namespace of the running pod. It is a variable so tests can
// point it at a temporary file.
var serviceAccountNamespacePath = "/var/run/secrets/kubernetes.io/serviceaccount/namespace"

// ResolveControllerNamespace returns the namespace of the running controller
// pod. The CONTROLLER_NAMESPACE environment variable takes precedence;
// otherwise the namespace is read from the service-account token projection.
// It returns an empty string when neither is available (e.g. when running
// outside a cluster), leaving rendered NetworkPolicies on their chart value
// defaults.
func ResolveControllerNamespace() string {
	if ns := os.Getenv(controllerNamespaceEnvVar); ns != "" {
		return ns
	}
	data, err := os.ReadFile(serviceAccountNamespacePath)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(data))
}
