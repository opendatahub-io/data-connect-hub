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
	"bufio"
	"encoding/json"
	"os/exec"
	"strings"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	appsv1 "k8s.io/api/apps/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/opendatahub-io/data-connect-hub/dc-controller/test/utils"
)

// AssertControllerJSONLogging verifies the controller JSON logging flag and schema.
func (e *E2EFixture) AssertControllerJSONLogging() {
	By("verifying controller JSON logging configuration")
	deployment := &appsv1.Deployment{}
	key := types.NamespacedName{Name: managerDeployment, Namespace: namespace}
	Expect(e.k8sClient.Get(e.ctx, key, deployment)).To(Succeed())
	assertContainerArg(deployment, "manager", "--zap-encoder=json")
	assertJSONLogRecord(namespace, managerDeployment, "manager")
}

// AssertServiceJSONLogging verifies the JSON logging flags and schemas rendered by DataConnectService.
func (e *E2EFixture) AssertServiceJSONLogging() {
	By("verifying REST and Flight JSON logging configuration")

	rest := &appsv1.Deployment{}
	Expect(e.k8sClient.Get(e.ctx, types.NamespacedName{
		Name: restDeploymentName, Namespace: applicationNamespace,
	}, rest)).To(Succeed())
	assertContainerArg(rest, "rest-server", "--json-logs")
	assertJSONLogRecord(applicationNamespace, restDeploymentName, "rest-server")

	flight := &appsv1.Deployment{}
	Expect(e.k8sClient.Get(e.ctx, types.NamespacedName{
		Name: e.flightServiceName, Namespace: applicationNamespace,
	}, flight)).To(Succeed())
	assertContainerArg(flight, "flight-server", "--json-logs")
	assertJSONLogRecord(applicationNamespace, e.flightServiceName, "flight-server")
}

func assertContainerArg(deployment *appsv1.Deployment, containerName, expected string) {
	for _, container := range deployment.Spec.Template.Spec.Containers {
		if container.Name != containerName {
			continue
		}
		if !contains(container.Args, expected) {
			Fail("expected " + expected + " in " + containerName + " arguments")
		}
		return
	}
	Fail("container " + containerName + " was not found in " + deployment.Name)
}

func assertJSONLogRecord(logNamespace, workload, container string) {
	var output string
	Eventually(func(g Gomega) {
		cmd := exec.Command("kubectl", "logs", "deployment/"+workload,
			"-n", logNamespace, "-c", container, "--tail=100")
		var err error
		output, err = utils.Run(cmd)
		g.Expect(err).NotTo(HaveOccurred())
		g.Expect(hasRequiredJSONLogRecord(output)).To(BeTrue(),
			"expected a JSON log record with ts, level, msg, logger, and service.name")
	}, "2m", "2s").Should(Succeed())
}

func hasRequiredJSONLogRecord(output string) bool {
	scanner := bufio.NewScanner(strings.NewReader(output))
	for scanner.Scan() {
		var record map[string]any
		if json.Unmarshal([]byte(scanner.Text()), &record) != nil {
			continue
		}
		for _, field := range []string{"ts", "level", "msg", "logger", "service.name"} {
			if _, ok := record[field]; !ok {
				goto nextRecord
			}
		}
		return true
	nextRecord:
	}
	return false
}

func contains(values []string, expected string) bool {
	for _, value := range values {
		if value == expected {
			return true
		}
	}
	return false
}
