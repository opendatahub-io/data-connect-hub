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
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"

	"github.com/opendatahub-io/data-connect-hub/dc-controller/test/utils"
)

var bearerAuthorizationPattern = regexp.MustCompile(`(?im)(authorization:\s*bearer\s+)[^\s]+`)

func redactMetricsOutput(output string) string {
	return bearerAuthorizationPattern.ReplaceAllString(output, "${1}[REDACTED]")
}

// AssertMetricsEndpoint verifies the controller metrics service returns HTTP 200.
func (e *E2EFixture) AssertMetricsEndpoint() {
	By("creating a ClusterRoleBinding for the service account to allow access to metrics")
	cmd := exec.Command("kubectl", "create", "clusterrolebinding", metricsRoleBindingName,
		"--clusterrole=dc-controller-metrics-reader",
		fmt.Sprintf("--serviceaccount=%s:%s", namespace, serviceAccountName),
	)
	_, err := utils.Run(cmd)
	Expect(err).NotTo(HaveOccurred(), "Failed to create ClusterRoleBinding")

	By("validating that the metrics service is available")
	cmd = exec.Command("kubectl", "get", "service", metricsServiceName, "-n", namespace)
	_, err = utils.Run(cmd)
	Expect(err).NotTo(HaveOccurred(), "Metrics service should exist")

	By("getting the service account token")
	token, err := serviceAccountToken()
	Expect(err).NotTo(HaveOccurred())
	Expect(token).NotTo(BeEmpty())

	By("ensuring the controller pod is ready")
	verifyControllerPodReady := func(g Gomega) {
		cmd := exec.Command("kubectl", "get", "pod", e.controllerPodName, "-n", namespace,
			"-o", "jsonpath={.status.conditions[?(@.type=='Ready')].status}")
		output, err := utils.Run(cmd)
		g.Expect(err).NotTo(HaveOccurred())
		g.Expect(output).To(Equal("True"), "Controller pod not ready")
	}
	Eventually(verifyControllerPodReady, 3*time.Minute, time.Second).Should(Succeed())

	By("verifying that the controller manager is serving the metrics server")
	verifyMetricsServerStarted := func(g Gomega) {
		cmd := exec.Command("kubectl", "logs", e.controllerPodName, "-n", namespace)
		output, err := utils.Run(cmd)
		g.Expect(err).NotTo(HaveOccurred())
		g.Expect(output).To(ContainSubstring("Serving metrics server"), "Metrics server not yet started")
	}
	Eventually(verifyMetricsServerStarted, 3*time.Minute, time.Second).Should(Succeed())

	By("creating the curl-metrics pod to access the metrics endpoint")
	cmd = exec.Command("kubectl", "run", "curl-metrics", "--restart=Never",
		"--namespace", namespace,
		"--image=curlimages/curl:latest",
		"--overrides",
		fmt.Sprintf(`{
			"spec": {
				"containers": [{
					"name": "curl",
					"image": "curlimages/curl:latest",
					"command": ["/bin/sh", "-c"],
					"args": [
						"for i in $(seq 1 30); do curl -v -k -H 'Authorization: Bearer %s' https://%s.%s.svc.cluster.local:8443/metrics && exit 0 || sleep 2; done; exit 1"
					],
					"securityContext": {
						"readOnlyRootFilesystem": true,
						"allowPrivilegeEscalation": false,
						"capabilities": {
							"drop": ["ALL"]
						},
						"runAsNonRoot": true,
						"runAsUser": 1000,
						"seccompProfile": {
							"type": "RuntimeDefault"
						}
					}
				}],
				"serviceAccountName": "%s"
			}
		}`, token, metricsServiceName, namespace, serviceAccountName))
	_, err = utils.Run(cmd)
	Expect(err).NotTo(HaveOccurred(), "Failed to create curl-metrics pod")

	By("waiting for the curl-metrics pod to complete")
	verifyCurlUp := func(g Gomega) {
		cmd := exec.Command("kubectl", "get", "pods", "curl-metrics",
			"-o", "jsonpath={.status.phase}",
			"-n", namespace)
		output, err := utils.Run(cmd)
		g.Expect(err).NotTo(HaveOccurred())
		g.Expect(output).To(Equal("Succeeded"), "curl pod in wrong status")
	}
	Eventually(verifyCurlUp, 5*time.Minute).Should(Succeed())

	By("getting the metrics by checking curl-metrics logs")
	verifyMetricsAvailable := func(g Gomega) {
		metricsOutput, err := getMetricsOutput()
		g.Expect(err).NotTo(HaveOccurred(), "Failed to retrieve logs from curl pod")
		metricsOutput = redactMetricsOutput(metricsOutput)
		g.Expect(metricsOutput).NotTo(BeEmpty())
		g.Expect(metricsOutput).To(ContainSubstring("< HTTP/1.1 200 OK"))
	}
	Eventually(verifyMetricsAvailable, 2*time.Minute).Should(Succeed())
}

// serviceAccountToken returns a token for the service account used to access metrics.
func serviceAccountToken() (string, error) {
	const tokenRequestRawString = `{
		"apiVersion": "authentication.k8s.io/v1",
		"kind": "TokenRequest"
	}`

	By("creating temporary file to store the token request")
	tokenRequestFile := filepath.Join("/tmp", fmt.Sprintf("%s-token-request", serviceAccountName))
	if err := os.WriteFile(tokenRequestFile, []byte(tokenRequestRawString), os.FileMode(0o644)); err != nil {
		return "", err
	}

	var out string
	verifyTokenCreation := func(g Gomega) {
		By("executing kubectl command to create the token")
		cmd := exec.Command("kubectl", "create", "--raw", fmt.Sprintf(
			"/api/v1/namespaces/%s/serviceaccounts/%s/token",
			namespace,
			serviceAccountName,
		), "-f", tokenRequestFile)

		output, err := cmd.CombinedOutput()
		g.Expect(err).NotTo(HaveOccurred())

		By("parsing the JSON output to extract the token")
		var token tokenRequest
		err = json.Unmarshal(output, &token)
		g.Expect(err).NotTo(HaveOccurred())

		out = token.Status.Token
	}
	Eventually(verifyTokenCreation).Should(Succeed())

	return out, nil
}

func getMetricsOutput() (string, error) {
	By("getting the curl-metrics logs")
	cmd := exec.Command("kubectl", "logs", "curl-metrics", "-n", namespace)
	return utils.Run(cmd)
}

type tokenRequest struct {
	Status struct {
		Token string `json:"token"`
	} `json:"status"`
}
