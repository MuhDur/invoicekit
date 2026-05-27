// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//go:build !cgo

package invoicekit

import (
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
)

// T-107: REST fallback unit test. The Go client is exercised
// against an in-process httptest server so this test does NOT
// require the rest-shim binary to be running on the host. It
// asserts request shape (path, method, headers, body) and that
// the response is decoded correctly.

func TestRESTFallbackPostsToConfiguredEndpointAndDecodesResponse(t *testing.T) {
	expectedBody := []byte(`{"abi_version":1,"operation":"engine.echo","payload":{}}`)
	expectedRespBytes := []byte(`{"status":"ok","echo":"hello"}`)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			t.Errorf("expected POST, got %s", r.Method)
		}
		if r.URL.Path != "/v1/engine/process_json" {
			t.Errorf("expected /v1/engine/process_json, got %s", r.URL.Path)
		}
		if got := r.Header.Get("Content-Type"); got != "application/json" {
			t.Errorf("expected Content-Type application/json, got %q", got)
		}
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatalf("read body: %v", err)
		}
		if string(body) != string(expectedBody) {
			t.Errorf("body mismatch: got %s", body)
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(restResponse{
			Status:         0,
			ResponseBase64: base64.StdEncoding.EncodeToString(expectedRespBytes),
		})
	}))
	defer server.Close()

	t.Setenv(envRestURL, server.URL)
	resp, status, err := Process(expectedBody)
	if err != nil {
		t.Fatal(err)
	}
	if status != 0 {
		t.Errorf("expected status 0, got %d", status)
	}
	if string(resp) != string(expectedRespBytes) {
		t.Errorf("response mismatch: got %s", resp)
	}
}

func TestRESTFallbackForwardsBearerTokenWhenConfigured(t *testing.T) {
	var receivedAuth string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		receivedAuth = r.Header.Get("Authorization")
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(restResponse{
			Status:         0,
			ResponseBase64: base64.StdEncoding.EncodeToString([]byte(`{"ok":true}`)),
		})
	}))
	defer server.Close()

	t.Setenv(envRestURL, server.URL)
	t.Setenv(envRestBearer, "test-token-7psv")
	_, _, err := Process([]byte(`{}`))
	if err != nil {
		t.Fatal(err)
	}
	if receivedAuth != "Bearer test-token-7psv" {
		t.Fatalf("expected Authorization header to carry bearer, got %q", receivedAuth)
	}
}

func TestRESTFallbackPropagatesNon2xxAsTypedError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "rate limited", http.StatusTooManyRequests)
	}))
	defer server.Close()

	t.Setenv(envRestURL, server.URL)
	_, _, err := Process([]byte(`{}`))
	if err == nil {
		t.Fatal("expected error on 429, got nil")
	}
	if !strings.Contains(err.Error(), "429") {
		t.Errorf("expected error to mention HTTP 429, got %q", err.Error())
	}
}

func TestRESTFallbackFailsCleanlyWhenEndpointUnreachable(t *testing.T) {
	// Bind to an unused localhost port and immediately close so
	// the next Dial gets connection refused. Using a fresh
	// httptest server then closing it gives us a deterministic
	// closed-port URL without touching the OS allocator.
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {}))
	closedURL := server.URL
	server.Close()

	t.Setenv(envRestURL, closedURL)
	_, _, err := Process([]byte(`{}`))
	if err == nil {
		t.Fatal("expected error on closed endpoint, got nil")
	}
	if !strings.Contains(err.Error(), "post") && !strings.Contains(err.Error(), "connection refused") {
		t.Logf("connection-refused phrasing varies by OS; got %q", err.Error())
	}
}

func TestRESTFallbackReportsTransportModeAsRest(t *testing.T) {
	if got := TransportMode(); got != "rest" {
		t.Fatalf("expected transport mode \"rest\" in nocgo build, got %q", got)
	}
}

// Sanity: the rest-fallback env-var defaults aren't accidentally
// overridden by a stray host environment when CI runs the matrix.
func TestRESTFallbackDefaultsAreNotLeakedFromHostEnv(t *testing.T) {
	// We don't t.Setenv here on purpose: this asserts what we
	// see when no env var is set.
	if _, ok := os.LookupEnv(envRestURL); ok {
		t.Skipf("%s is set in host env, skipping default-endpoint check", envRestURL)
	}
	// With no env var, the default endpoint is the documented
	// localhost. We just check the constant hasn't drifted.
	if defaultRestEndpoint != "http://127.0.0.1:8081" {
		t.Fatalf("default endpoint drifted: %q", defaultRestEndpoint)
	}
}
