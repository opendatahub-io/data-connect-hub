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
	"context"
	"fmt"
	"slices"
	"strings"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/discovery"
	"k8s.io/client-go/rest"
	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// Global namespaces are RHOAI-wide shared projects designated by a Namespace
// label. The controller watches that label, grants the platform Auth CR groups
// access to DCH resources in every labeled namespace, and makes their
// connection types visible to every tenant.
const (
	// GlobalNamespaceLabel is the label the Global Data Registry design
	// specifies for the Dashboard to apply to the designated global project.
	// TODO: the Dashboard does not apply this label yet; until it does, label
	// the namespace manually.
	GlobalNamespaceLabel = "opendatahub.io/global-data-registry-project"

	platformAuthCRName = "auth"

	// labelGlobalNamespace marks the RoleBindings the controller owns in the
	// global namespace so they can be found for cleanup and drift repair.
	labelGlobalNamespace = "dataconnecthub.opendatahub.io/global-namespace"
	managedByGlobalNS    = "global-namespace"

	roleBindingGlobalRead               = "dch-global-read"
	roleBindingGlobalReadWrite          = "dch-global-read-write"
	roleBindingGlobalRestSecretAccess   = "dch-global-rest-service-secret-access"
	roleBindingGlobalFlightSecretAccess = "dch-global-flight-service-secret-access"
	clusterRoleRead                     = "dch-read"
	clusterRoleReadWrite                = "dch-read-write"
	clusterRoleRestSecretAccess         = "dch-rest-service-secret-access"
	clusterRoleFlightSecretAccess       = "dch-flight-service-secret-access"
	serviceAccountRestService           = "dch-rest-service-sa"
	globalNSReconcileKeyStr             = "global-namespace"
	globalNSFieldOwner                  = "dc-controller-global-namespace"
)

var platformAuthGVK = schema.GroupVersionKind{
	Group:   "services.platform.opendatahub.io",
	Version: "v1alpha1",
	Kind:    "Auth",
}

// managedGlobalRoleBindings lists every RoleBinding name the controller may
// create in a global namespace. Bindings with other names are never touched,
// so administrators can add their own (e.g. a curator group bound to
// dch-read-write) without the controller removing them.
var managedGlobalRoleBindings = []string{
	roleBindingGlobalRead,
	roleBindingGlobalReadWrite,
	roleBindingGlobalRestSecretAccess,
	roleBindingGlobalFlightSecretAccess,
}

// GlobalNamespaceConfig is the resolved feature configuration shared by the
// controllers that need to know about the global namespace.
type GlobalNamespaceConfig struct {
	Enabled  bool
	LabelKey string
}

// NewGlobalNamespaceConfig enables the feature when the platform Auth CRD is
// installed, since the controller grants access from the Auth CR groups.
func NewGlobalNamespaceConfig(authAvailable func() (bool, error)) (GlobalNamespaceConfig, error) {
	available, err := authAvailable()
	if err != nil {
		return GlobalNamespaceConfig{}, fmt.Errorf("checking for the platform %s CRD: %w", platformAuthGVK.GroupKind(), err)
	}
	return GlobalNamespaceConfig{Enabled: available, LabelKey: GlobalNamespaceLabel}, nil
}

// PlatformAuthCRDAvailable reports whether the platform Auth API is served.
func PlatformAuthCRDAvailable(cfg *rest.Config) func() (bool, error) {
	return func() (bool, error) {
		dc, err := discovery.NewDiscoveryClientForConfig(cfg)
		if err != nil {
			return false, err
		}
		resources, err := dc.ServerResourcesForGroupVersion(platformAuthGVK.GroupVersion().String())
		if err != nil {
			if apierrors.IsNotFound(err) {
				return false, nil
			}
			return false, err
		}
		for _, r := range resources.APIResources {
			if r.Kind == platformAuthGVK.Kind {
				return true, nil
			}
		}
		return false, nil
	}
}

// labeledGlobalNamespaces lists the labeled, non-terminating Namespaces,
// sorted. Every one of them is a global namespace.
func labeledGlobalNamespaces(ctx context.Context, c client.Reader, gn GlobalNamespaceConfig) ([]string, error) {
	if !gn.Enabled {
		return nil, nil
	}
	var list corev1.NamespaceList
	if err := c.List(ctx, &list, client.HasLabels{gn.LabelKey}); err != nil {
		return nil, fmt.Errorf("listing namespaces labeled %s: %w", gn.LabelKey, err)
	}
	labeled := make([]string, 0, len(list.Items))
	for i := range list.Items {
		ns := &list.Items[i]
		if !ns.DeletionTimestamp.IsZero() {
			continue
		}
		labeled = append(labeled, ns.Name)
	}
	slices.Sort(labeled)
	return labeled, nil
}

// globalTenants returns the tenants whose connection types are visible to
// every tenant: the DataConnectService namespace, which holds the built-in
// connection types, followed by every labeled global namespace.
func globalTenants(serviceNamespace string, labeled []string) []string {
	tenants := []string{serviceNamespace}
	for _, ns := range labeled {
		if ns != serviceNamespace {
			tenants = append(tenants, ns)
		}
	}
	return tenants
}

// authGroups reads allowedGroups and adminGroups from the platform Auth CR.
func authGroups(ctx context.Context, c client.Reader) (allowed, admin []string, err error) {
	auth := &unstructured.Unstructured{}
	auth.SetGroupVersionKind(platformAuthGVK)
	if err := c.Get(ctx, types.NamespacedName{Name: platformAuthCRName}, auth); err != nil {
		return nil, nil, err
	}
	allowed, _, err = unstructured.NestedStringSlice(auth.Object, "spec", "allowedGroups")
	if err != nil {
		return nil, nil, fmt.Errorf("reading Auth spec.allowedGroups: %w", err)
	}
	admin, _, err = unstructured.NestedStringSlice(auth.Object, "spec", "adminGroups")
	if err != nil {
		return nil, nil, fmt.Errorf("reading Auth spec.adminGroups: %w", err)
	}
	return allowed, admin, nil
}

func groupSubjects(groups []string) []rbacv1.Subject {
	subjects := make([]rbacv1.Subject, 0, len(groups))
	for _, g := range groups {
		subjects = append(subjects, rbacv1.Subject{APIGroup: rbacv1.GroupName, Kind: rbacv1.GroupKind, Name: g})
	}
	return subjects
}

// serviceAccountRef identifies a DCH service ServiceAccount.
type serviceAccountRef struct {
	Namespace string
	Name      string
}

func serviceAccountSubjects(refs []serviceAccountRef) []rbacv1.Subject {
	subjects := make([]rbacv1.Subject, 0, len(refs))
	for _, ref := range refs {
		subjects = append(subjects, rbacv1.Subject{Kind: rbacv1.ServiceAccountKind, Namespace: ref.Namespace, Name: ref.Name})
	}
	slices.SortFunc(subjects, func(a, b rbacv1.Subject) int {
		return strings.Compare(a.Namespace+"/"+a.Name, b.Namespace+"/"+b.Name)
	})
	return subjects
}

// flightServiceAccountName mirrors how renderFlightService renames the base
// "dch-flight-service-sa" ServiceAccount for a DataConnectService.
func flightServiceAccountName(crName string) string {
	return "dch-" + flightServiceResourceName(crName) + "-sa"
}

// serviceAccountsFor lists the REST and Flight ServiceAccounts of every
// DataConnectService; they need Secret access in the global namespaces to
// store and read connection credentials there.
func serviceAccountsFor(items []dchv1alpha1.DataConnectService) (restSAs, flightSAs []serviceAccountRef) {
	for i := range items {
		cr := &items[i]
		if !cr.DeletionTimestamp.IsZero() {
			continue
		}
		restSAs = append(restSAs, serviceAccountRef{Namespace: cr.Namespace, Name: serviceAccountRestService})
		flightSAs = append(flightSAs, serviceAccountRef{Namespace: cr.Namespace, Name: flightServiceAccountName(cr.Name)})
	}
	return restSAs, flightSAs
}

// uniqueSorted merges the lists, then sorts and dedupes the result so the
// rendered subjects are stable across reconciles.
func uniqueSorted(values ...[]string) []string {
	out := slices.Concat(values...)
	slices.Sort(out)
	return slices.Compact(out)
}

// desiredGlobalRoleBindings renders the RoleBindings for a global namespace.
// Every managed binding is rendered, even with no subjects; such a binding
// grants nothing.
func desiredGlobalRoleBindings(namespace string, allowed, admin []string, restSAs, flightSAs []serviceAccountRef) []*rbacv1.RoleBinding {
	readers := uniqueSorted(allowed, admin)
	writers := uniqueSorted(admin)

	specs := []struct {
		name     string
		role     string
		subjects []rbacv1.Subject
	}{
		{roleBindingGlobalRead, clusterRoleRead, groupSubjects(readers)},
		{roleBindingGlobalReadWrite, clusterRoleReadWrite, groupSubjects(writers)},
		{roleBindingGlobalRestSecretAccess, clusterRoleRestSecretAccess, serviceAccountSubjects(restSAs)},
		{roleBindingGlobalFlightSecretAccess, clusterRoleFlightSecretAccess, serviceAccountSubjects(flightSAs)},
	}

	out := make([]*rbacv1.RoleBinding, 0, len(specs))
	for _, s := range specs {
		rb := &rbacv1.RoleBinding{}
		rb.SetGroupVersionKind(rbacv1.SchemeGroupVersion.WithKind(kindRoleBinding))
		rb.Name = s.name
		rb.Namespace = namespace
		rb.Labels = map[string]string{
			labelManagedBy:       managedByGlobalNS,
			labelGlobalNamespace: valueTrue,
			labelPartOf:          nameDataConnectHub,
		}
		rb.RoleRef = rbacv1.RoleRef{APIGroup: rbacv1.GroupName, Kind: kindClusterRole, Name: s.role}
		rb.Subjects = s.subjects
		out = append(out, rb)
	}
	return out
}
