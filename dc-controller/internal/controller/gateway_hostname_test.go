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
	"testing"

	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/fake"
	"sigs.k8s.io/controller-runtime/pkg/client/interceptor"

	dchv1alpha1 "github.com/opendatahub-io/data-connect-hub/dc-controller/api/dataconnecthub/v1alpha1"
)

// Fixture values, kept as constants to stay under the goconst threshold.
const (
	nameKey     = "name"
	hostnameKey = "hostname"
	valueKey    = "value"

	httpsListener   = "https"
	metricsListener = "metrics"

	testDCSName = "test-dcs"

	// The listener hostname a LoadBalancer-mode RHOAI gateway publishes --
	// correct for both SNI and DNS.
	testListenerHostname = "rh-ai.apps.example.com"
	// A wildcard listener hostname: describes what the listener matches, not
	// an address a client can connect to.
	testWildcardListenerHostname = "*.example.com"
	// The router-assigned host of the Route an OcpRoute-mode gateway is
	// published through.
	testRouteHostname = "data-science-gateway-openshift-ingress.apps.example.com"
	// The raw ELB hostname Gateway status carries in LoadBalancer mode --
	// usable as a DNS target but not as an SNI.
	testELBHostname = "a1b2c3d4.elb.us-east-1.amazonaws.com"
)

var (
	gatewayGVK = schema.GroupVersionKind{
		Group:   "gateway.networking.k8s.io",
		Version: "v1",
		Kind:    "Gateway",
	}
	routeGVK = schema.GroupVersionKind{
		Group:   "route.openshift.io",
		Version: "v1",
		Kind:    "Route",
	}
)

// hostnameTestClient returns a fake client serving the unstructured Gateway
// and Route objects resolveGatewayHostname reads.
func hostnameTestClient(objects ...client.Object) client.Client {
	return hostnameTestClientWith(interceptor.Funcs{}, objects...)
}

// hostnameTestClientWith additionally intercepts client calls, for failing
// lookups.
func hostnameTestClientWith(funcs interceptor.Funcs, objects ...client.Object) client.Client {
	scheme := runtime.NewScheme()
	for _, gvk := range []schema.GroupVersionKind{gatewayGVK, routeGVK} {
		scheme.AddKnownTypeWithName(gvk, &unstructured.Unstructured{})
		scheme.AddKnownTypeWithName(gvk.GroupVersion().WithKind(gvk.Kind+"List"), &unstructured.UnstructuredList{})
	}
	return fake.NewClientBuilder().WithScheme(scheme).WithInterceptorFuncs(funcs).WithObjects(objects...).Build()
}

// failKindGet makes Gets of the given kind return the supplied error.
func failKindGet(kind string, err error) interceptor.Funcs {
	return interceptor.Funcs{
		Get: func(ctx context.Context, c client.WithWatch, key client.ObjectKey, obj client.Object, opts ...client.GetOption) error {
			if u, ok := obj.(*unstructured.Unstructured); ok && u.GetKind() == kind {
				return err
			}
			return c.Get(ctx, key, obj, opts...)
		},
	}
}

func testGateway(listeners []any, statusAddresses []any) *unstructured.Unstructured {
	gw := &unstructured.Unstructured{}
	gw.SetGroupVersionKind(gatewayGVK)
	gw.SetName(testGatewayName)
	gw.SetNamespace(testOpenShiftGatewayNamespace)
	if listeners != nil {
		gw.Object["spec"] = map[string]any{"listeners": listeners}
	}
	if statusAddresses != nil {
		gw.Object["status"] = map[string]any{"addresses": statusAddresses}
	}
	return gw
}

func testRoute(host string) *unstructured.Unstructured {
	route := &unstructured.Unstructured{}
	route.SetGroupVersionKind(routeGVK)
	route.SetName(testGatewayName)
	route.SetNamespace(testOpenShiftGatewayNamespace)
	route.Object["spec"] = map[string]any{"host": host}
	return route
}

func elbAddress() []any {
	return []any{map[string]any{"type": "Hostname", valueKey: testELBHostname}}
}

// mustResolve asserts the lookup succeeds and returns its hostname.
func mustResolve(t *testing.T, r *DataConnectServiceReconciler) string {
	t.Helper()
	hostname, err := r.resolveGatewayHostname(context.Background(), testOpenShiftGatewayNamespace, testGatewayName)
	if err != nil {
		t.Fatalf("resolveGatewayHostname() unexpected error: %v", err)
	}
	return hostname
}

func TestResolveGatewayHostnamePrefersListenerHostname(t *testing.T) {
	// LoadBalancer mode: the listener hostname (correct SNI and DNS) must win
	// over the raw ELB address in status.addresses.
	r := &DataConnectServiceReconciler{Client: hostnameTestClient(testGateway(
		[]any{map[string]any{nameKey: httpsListener, hostnameKey: testListenerHostname}},
		elbAddress(),
	))}

	if hostname := mustResolve(t, r); hostname != testListenerHostname {
		t.Fatalf("resolveGatewayHostname() = %q, want listener hostname %s", hostname, testListenerHostname)
	}
}

func TestResolveGatewayHostnameSkipsListenersWithoutHostname(t *testing.T) {
	r := &DataConnectServiceReconciler{Client: hostnameTestClient(testGateway(
		[]any{
			map[string]any{nameKey: metricsListener},
			map[string]any{nameKey: httpsListener, hostnameKey: testListenerHostname},
		},
		elbAddress(),
	))}

	if hostname := mustResolve(t, r); hostname != testListenerHostname {
		t.Fatalf("resolveGatewayHostname() = %q, want first non-empty listener hostname", hostname)
	}
}

func TestResolveGatewayHostnameSkipsWildcardListener(t *testing.T) {
	// A wildcard listener hostname is not a connectable address, so the chain
	// must fall through to the Route (and, below it, the status fallback,
	// which is connectable precisely because a wildcard listener accepts any
	// SNI).
	r := &DataConnectServiceReconciler{Client: hostnameTestClient(
		testGateway([]any{map[string]any{nameKey: httpsListener, hostnameKey: testWildcardListenerHostname}}, nil),
		testRoute(testRouteHostname),
	)}

	if hostname := mustResolve(t, r); hostname != testRouteHostname {
		t.Fatalf("resolveGatewayHostname() = %q, want Route spec.host %s", hostname, testRouteHostname)
	}
}

func TestResolveGatewayHostnamePrefersConcreteListenerOverWildcard(t *testing.T) {
	r := &DataConnectServiceReconciler{Client: hostnameTestClient(testGateway(
		[]any{
			map[string]any{nameKey: httpsListener, hostnameKey: testWildcardListenerHostname},
			map[string]any{nameKey: metricsListener, hostnameKey: testListenerHostname},
		},
		elbAddress(),
	))}

	if hostname := mustResolve(t, r); hostname != testListenerHostname {
		t.Fatalf("resolveGatewayHostname() = %q, want the concrete listener hostname %s", hostname, testListenerHostname)
	}
}

func TestResolveGatewayHostnameFallsBackToRouteHost(t *testing.T) {
	// OcpRoute mode: listeners carry no hostname and the platform operator
	// publishes the gateway through a Route sharing its name and namespace.
	r := &DataConnectServiceReconciler{Client: hostnameTestClient(
		testGateway([]any{map[string]any{nameKey: httpsListener}}, nil),
		testRoute(testRouteHostname),
	)}

	if hostname := mustResolve(t, r); hostname != testRouteHostname {
		t.Fatalf("resolveGatewayHostname() = %q, want Route spec.host", hostname)
	}
}

func TestResolveGatewayHostnameFallsBackToStatusAddress(t *testing.T) {
	// Neither a listener hostname nor a Route (the Route Get returns
	// NotFound, which must be tolerated): keep the legacy status.addresses
	// value so the field is not silently emptied.
	r := &DataConnectServiceReconciler{Client: hostnameTestClient(
		testGateway([]any{map[string]any{nameKey: httpsListener}}, elbAddress()),
	)}

	if hostname := mustResolve(t, r); hostname != testELBHostname {
		t.Fatalf("resolveGatewayHostname() = %q, want status.addresses[0].value", hostname)
	}
}

func TestResolveGatewayHostnameEmptyWhenGatewayMissing(t *testing.T) {
	r := &DataConnectServiceReconciler{Client: hostnameTestClient()}

	hostname, err := r.resolveGatewayHostname(context.Background(), testOpenShiftGatewayNamespace, testGatewayName)
	if err != nil {
		t.Fatalf("resolveGatewayHostname() error = %v, want nil for a missing Gateway", err)
	}
	if hostname != "" {
		t.Fatalf("resolveGatewayHostname() = %q, want empty for missing Gateway", hostname)
	}
}

func TestResolveGatewayHostnameToleratesForbiddenRouteLookup(t *testing.T) {
	// The documented Helm degrade case: a chart whose ClusterRole does not
	// grant Route reads. The lookup must fall through to the status address
	// rather than fail the reconcile.
	r := &DataConnectServiceReconciler{Client: hostnameTestClientWith(
		failKindGet(routeGVK.Kind, apierrors.NewForbidden(schema.GroupResource{Group: routeGVK.Group, Resource: "routes"}, testGatewayName, nil)),
		testGateway([]any{map[string]any{nameKey: httpsListener}}, elbAddress()),
	)}

	if hostname := mustResolve(t, r); hostname != testELBHostname {
		t.Fatalf("resolveGatewayHostname() = %q, want status.addresses[0].value despite Forbidden Route read", hostname)
	}
}

func TestResolveGatewayHostnamePropagatesRouteLookupError(t *testing.T) {
	// A transient Route lookup failure must not be treated as an absent
	// Route: falling through would publish the raw status address as if it
	// were the resolved hostname.
	r := &DataConnectServiceReconciler{Client: hostnameTestClientWith(
		failKindGet(routeGVK.Kind, apierrors.NewTimeoutError("route lookup timed out", 0)),
		testGateway([]any{map[string]any{nameKey: httpsListener}}, elbAddress()),
	)}

	hostname, err := r.resolveGatewayHostname(context.Background(), testOpenShiftGatewayNamespace, testGatewayName)
	if err == nil {
		t.Fatalf("resolveGatewayHostname() = %q, want an error for a transient Route lookup failure", hostname)
	}
}

func TestResolveGatewayHostnamePropagatesGatewayLookupError(t *testing.T) {
	r := &DataConnectServiceReconciler{Client: hostnameTestClientWith(
		failKindGet(gatewayGVK.Kind, apierrors.NewTimeoutError("gateway lookup timed out", 0)),
	)}

	hostname, err := r.resolveGatewayHostname(context.Background(), testOpenShiftGatewayNamespace, testGatewayName)
	if err == nil {
		t.Fatalf("resolveGatewayHostname() = %q, want an error for a transient Gateway lookup failure", hostname)
	}
}

func TestGatewayStatusPublishesListenerHostname(t *testing.T) {
	// The resolved hostname must land in the public CR status that
	// dashboards, the SDK and automation consume.
	r := &DataConnectServiceReconciler{Client: hostnameTestClient(testGateway(
		[]any{map[string]any{nameKey: httpsListener, hostnameKey: testListenerHostname}},
		elbAddress(),
	))}
	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: testDCSName, Namespace: testNamespace},
	}
	platCfg := &platformConfig{GatewayName: testGatewayName, GatewayNamespace: testOpenShiftGatewayNamespace}

	if err := r.gatewayStatus(context.Background(), cr, platCfg); err != nil {
		t.Fatalf("gatewayStatus() error = %v, want nil", err)
	}

	if len(cr.Status.Addresses) != 1 {
		t.Fatalf("Status.Addresses = %+v, want exactly one entry", cr.Status.Addresses)
	}
	addr := cr.Status.Addresses[0]
	if addr.Type != addressTypeHostname || addr.Value != testListenerHostname {
		t.Fatalf("Status.Addresses[0] = %+v, want hostname %s", addr, testListenerHostname)
	}
	if cr.Status.Gateway == nil || cr.Status.Gateway.Name != testGatewayName || cr.Status.Gateway.Namespace != testOpenShiftGatewayNamespace {
		t.Fatalf("Status.Gateway = %+v, want %s/%s", cr.Status.Gateway, testOpenShiftGatewayNamespace, testGatewayName)
	}
}

func TestGatewayStatusOmitsAddressWhenUnresolvable(t *testing.T) {
	// Nothing resolves (no Gateway, no Route, nothing published): the CR must
	// carry no address entry at all rather than one with an empty value, and
	// a previously resolved address must not survive.
	r := &DataConnectServiceReconciler{Client: hostnameTestClient()}
	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: testDCSName, Namespace: testNamespace},
	}
	cr.Status.Addresses = []dchv1alpha1.Addresses{{Type: addressTypeHostname, Value: "stale.example.com"}}
	platCfg := &platformConfig{GatewayName: testGatewayName, GatewayNamespace: testOpenShiftGatewayNamespace}

	if err := r.gatewayStatus(context.Background(), cr, platCfg); err != nil {
		t.Fatalf("gatewayStatus() error = %v, want nil", err)
	}

	if len(cr.Status.Addresses) != 0 {
		t.Fatalf("Status.Addresses = %+v, want no entries when no hostname resolves", cr.Status.Addresses)
	}
	if cr.Status.Gateway == nil {
		t.Fatal("Status.Gateway = nil, want the gateway reference even without an address")
	}
}

func TestGatewayStatusKeepsLastAddressOnLookupError(t *testing.T) {
	// A transient lookup failure must leave the last-known address in place
	// and report the error so the caller retries.
	r := &DataConnectServiceReconciler{Client: hostnameTestClientWith(
		failKindGet(gatewayGVK.Kind, apierrors.NewTimeoutError("gateway lookup timed out", 0)),
	)}
	cr := &dchv1alpha1.DataConnectService{
		ObjectMeta: metav1.ObjectMeta{Name: testDCSName, Namespace: testNamespace},
	}
	cr.Status.Addresses = []dchv1alpha1.Addresses{{Type: addressTypeHostname, Value: testListenerHostname}}
	platCfg := &platformConfig{GatewayName: testGatewayName, GatewayNamespace: testOpenShiftGatewayNamespace}

	err := r.gatewayStatus(context.Background(), cr, platCfg)
	if err == nil {
		t.Fatal("gatewayStatus() = nil error, want the lookup failure propagated")
	}
	if len(cr.Status.Addresses) != 1 || cr.Status.Addresses[0].Value != testListenerHostname {
		t.Fatalf("Status.Addresses = %+v, want the last-known address preserved", cr.Status.Addresses)
	}
}
