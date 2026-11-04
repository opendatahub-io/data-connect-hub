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
	"os"
	"slices"
	"strconv"
	"strings"

	corev1 "k8s.io/api/core/v1"
	rbacv1 "k8s.io/api/rbac/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/meta"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/discovery"
	"k8s.io/client-go/rest"
	"sigs.k8s.io/controller-runtime/pkg/client"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// The global namespace is the single RHOAI-wide shared project designated
// from the Dashboard. The Dashboard labels the designated Namespace; the
// controller watches that label, grants the platform Auth CR groups access to
// DCH resources in it, and uses it as the tenant for global connection types.
const (
	// EnvEnableGlobalNamespace controls the global namespace feature:
	// "true" requires the platform Auth CRD, "false" disables the feature, and
	// an empty value enables it only when the Auth CRD is installed.
	EnvEnableGlobalNamespace = "ENABLE_GLOBAL_NAMESPACE"
	// EnvGlobalNamespaceLabel overrides the Namespace label key that designates
	// the global namespace.
	EnvGlobalNamespaceLabel = "GLOBAL_NAMESPACE_LABEL"
	// DefaultGlobalNamespaceLabel is the label the Global Data Registry design
	// specifies for the Dashboard to apply to the designated global project.
	// TODO: the Dashboard does not apply this label yet; it only applies
	// opendatahub.io/global-mlflow-workspace. Follow up with the Dashboard team,
	// and until then label the namespace manually or set GLOBAL_NAMESPACE_LABEL.
	DefaultGlobalNamespaceLabel = "opendatahub.io/global-data-registry-project"

	authCRName = "auth"

	// labelGlobalNamespace marks the RoleBindings the controller owns in the
	// global namespace so they can be found for cleanup and drift repair.
	labelGlobalNamespace = "dataconnecthub.opendatahub.io/global-namespace"
	managedByGlobalNS    = "global-namespace"

	roleBindingGlobalRead          = "dch-global-read"
	roleBindingGlobalReadWrite     = "dch-global-read-write"
	roleBindingGlobalRestSecret    = "dch-global-rest-secret-access"
	roleBindingGlobalFlightSecret  = "dch-global-flight-secret-read"
	clusterRoleRead                = "dch-read"
	clusterRoleReadWrite           = "dch-read-write"
	clusterRoleRestSecretAccess    = "dch-rest-service-secret-access"
	clusterRoleFlightSecretRead    = "dch-flight-secret-read"
	serviceAccountRestService      = "dch-rest-service-sa"
	conditionTypeGlobalNSReady     = "GlobalNamespaceReady"
	reasonGlobalNSReconciled       = "Reconciled"
	reasonGlobalNSNotConfigured    = "NotConfigured"
	reasonGlobalNSMultiple         = "MultipleGlobalNamespaces"
	reasonGlobalNSAuthNotFound     = "AuthNotFound"
	reasonGlobalNSReconcileFailed  = "ReconcileFailed"
	globalNamespaceFieldOwner      = "dc-controller-global-namespace"
	globalNamespaceReconcileKeyStr = "global-namespace"
)

var authGVK = schema.GroupVersionKind{
	Group:   "services.platform.opendatahub.io",
	Version: "v1alpha1",
	Kind:    "Auth",
}

// managedGlobalRoleBindings lists every RoleBinding name the controller may
// create in the global namespace. Bindings with other names are never touched,
// so administrators can add their own (e.g. a curator group bound to
// dch-read-write) without the controller removing them.
var managedGlobalRoleBindings = []string{
	roleBindingGlobalRead,
	roleBindingGlobalReadWrite,
	roleBindingGlobalRestSecret,
	roleBindingGlobalFlightSecret,
}

// GlobalNamespaceConfig is the resolved feature configuration shared by the
// controllers that need to know about the global namespace.
type GlobalNamespaceConfig struct {
	Enabled  bool
	LabelKey string
}

// LoadGlobalNamespaceConfig reads the feature configuration from the
// environment. When the feature flag is unset, authAvailable decides.
func LoadGlobalNamespaceConfig(authAvailable func() (bool, error)) (GlobalNamespaceConfig, error) {
	labelKey := strings.TrimSpace(os.Getenv(EnvGlobalNamespaceLabel))
	if labelKey == "" {
		labelKey = DefaultGlobalNamespaceLabel
	}
	cfg := GlobalNamespaceConfig{LabelKey: labelKey}

	raw := strings.TrimSpace(os.Getenv(EnvEnableGlobalNamespace))
	if raw != "" {
		enabled, err := strconv.ParseBool(raw)
		if err != nil {
			return cfg, fmt.Errorf("invalid %s value %q: %w", EnvEnableGlobalNamespace, raw, err)
		}
		if !enabled {
			return cfg, nil
		}
	}

	available, err := authAvailable()
	if err != nil {
		return cfg, fmt.Errorf("checking for the platform Auth CRD: %w", err)
	}
	if !available {
		if raw != "" {
			return cfg, fmt.Errorf("%s=true but the %s CRD is not installed", EnvEnableGlobalNamespace, authGVK.GroupKind())
		}
		return cfg, nil
	}
	cfg.Enabled = true
	return cfg, nil
}

// AuthCRDAvailable reports whether the platform Auth API is served.
func AuthCRDAvailable(cfg *rest.Config) func() (bool, error) {
	return func() (bool, error) {
		dc, err := discovery.NewDiscoveryClientForConfig(cfg)
		if err != nil {
			return false, err
		}
		resources, err := dc.ServerResourcesForGroupVersion(authGVK.GroupVersion().String())
		if err != nil {
			if apierrors.IsNotFound(err) {
				return false, nil
			}
			return false, err
		}
		for _, r := range resources.APIResources {
			if r.Kind == authGVK.Kind {
				return true, nil
			}
		}
		return false, nil
	}
}

// globalNamespaceResolution is the outcome of looking up labeled Namespaces.
type globalNamespaceResolution struct {
	// Effective is the namespace in use: the single labeled namespace, the
	// previously effective one when the label is ambiguous, or empty.
	Effective string
	// Labeled lists every namespace that carries the label, sorted.
	Labeled []string
}

// Ambiguous reports whether more than one namespace carries the label.
func (r globalNamespaceResolution) Ambiguous() bool {
	return len(r.Labeled) > 1
}

// resolveGlobalNamespace lists labeled, non-terminating Namespaces. With more
// than one, it keeps previous so a stray label cannot silently move the global
// tenant or grant access in an unintended namespace.
func resolveGlobalNamespace(ctx context.Context, c client.Reader, gn GlobalNamespaceConfig, previous string) (globalNamespaceResolution, error) {
	if !gn.Enabled {
		return globalNamespaceResolution{}, nil
	}
	var list corev1.NamespaceList
	if err := c.List(ctx, &list, client.HasLabels{gn.LabelKey}); err != nil {
		return globalNamespaceResolution{}, fmt.Errorf("listing namespaces labeled %s: %w", gn.LabelKey, err)
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

	res := globalNamespaceResolution{Labeled: labeled}
	switch len(labeled) {
	case 0:
	case 1:
		res.Effective = labeled[0]
	default:
		if slices.Contains(labeled, previous) {
			res.Effective = previous
		}
	}
	return res, nil
}

// globalTenantFor returns the tenant that global connection types belong to
// for a DataConnectService: the effective global namespace, or the CR
// namespace when none is designated.
func globalTenantFor(ctx context.Context, c client.Reader, gn GlobalNamespaceConfig, cr *dchv1alpha1.DataConnectService) (string, error) {
	res, err := resolveGlobalNamespace(ctx, c, gn, cr.Status.GlobalNamespace)
	if err != nil {
		return "", err
	}
	if res.Effective != "" {
		return res.Effective, nil
	}
	return cr.Namespace, nil
}

// connectionTypeTenantFor maps the namespace a connection type source lives in
// to the tenant it registers under. Sources in a DataConnectService namespace
// (InitDataConnectionTypes and promoted ODH ConfigMaps) define global
// connection types, so they follow the global tenant; any other namespace is
// its own tenant.
func connectionTypeTenantFor(ctx context.Context, c client.Reader, gn GlobalNamespaceConfig, namespace string) (string, error) {
	if !gn.Enabled {
		return namespace, nil
	}
	var list dchv1alpha1.DataConnectServiceList
	if err := c.List(ctx, &list, client.InNamespace(namespace)); err != nil {
		if meta.IsNoMatchError(err) {
			return namespace, nil
		}
		return "", fmt.Errorf("listing DataConnectService in %s: %w", namespace, err)
	}
	if len(list.Items) == 0 {
		return namespace, nil
	}
	return globalTenantFor(ctx, c, gn, &list.Items[0])
}

// authGroups reads allowedGroups and adminGroups from the platform Auth CR.
func authGroups(ctx context.Context, c client.Reader) (allowed, admin []string, err error) {
	auth := &unstructured.Unstructured{}
	auth.SetGroupVersionKind(authGVK)
	if err := c.Get(ctx, types.NamespacedName{Name: authCRName}, auth); err != nil {
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

// uniqueSorted drops empty and duplicate entries and sorts the rest so the
// rendered subjects are stable across reconciles.
func uniqueSorted(values ...[]string) []string {
	var out []string
	for _, list := range values {
		for _, v := range list {
			v = strings.TrimSpace(v)
			if v != "" && !slices.Contains(out, v) {
				out = append(out, v)
			}
		}
	}
	slices.Sort(out)
	return out
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
// DataConnectService; they need Secret access in the global namespace to
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

// desiredGlobalRoleBindings renders the RoleBindings for the global namespace.
// A binding whose subject list is empty is omitted so it gets removed rather
// than left granting nothing.
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
		{roleBindingGlobalRestSecret, clusterRoleRestSecretAccess, serviceAccountSubjects(restSAs)},
		{roleBindingGlobalFlightSecret, clusterRoleFlightSecretRead, serviceAccountSubjects(flightSAs)},
	}

	out := make([]*rbacv1.RoleBinding, 0, len(specs))
	for _, s := range specs {
		if len(s.subjects) == 0 {
			continue
		}
		rb := &rbacv1.RoleBinding{}
		rb.SetGroupVersionKind(rbacv1.SchemeGroupVersion.WithKind(kindRoleBinding))
		rb.Name = s.name
		rb.Namespace = namespace
		rb.Labels = map[string]string{
			labelManagedBy:              managedByGlobalNS,
			labelGlobalNamespace:        valueTrue,
			"app.kubernetes.io/part-of": nameDataConnectHub,
		}
		rb.RoleRef = rbacv1.RoleRef{APIGroup: rbacv1.GroupName, Kind: kindClusterRole, Name: s.role}
		rb.Subjects = s.subjects
		out = append(out, rb)
	}
	return out
}
