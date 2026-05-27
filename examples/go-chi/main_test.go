// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1407 smoke test: exercises every route via net/http/httptest
// so the gate stays fast and matches the FastAPI / Django demos'
// 6-row layout.

package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestIndexReturnsFixtureList(t *testing.T) {
	w := httptest.NewRecorder()
	newRouter().ServeHTTP(w, httptest.NewRequest(http.MethodGet, "/", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}
	var body map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if title, _ := body["title"].(string); title != "InvoiceKit Go (chi) demo" {
		t.Fatalf("unexpected title: %v", body["title"])
	}
	fixtures, _ := body["fixtures"].([]any)
	if len(fixtures) != 3 {
		t.Fatalf("expected 3 fixtures, got %d", len(fixtures))
	}
}

func TestHealthz(t *testing.T) {
	w := httptest.NewRecorder()
	newRouter().ServeHTTP(w, httptest.NewRequest(http.MethodGet, "/healthz", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", w.Code)
	}
}

func TestCanonicalizeBasicFixture(t *testing.T) {
	requireCanonicalize(t, "basic")
}

func TestCanonicalizeWithAllowanceFixture(t *testing.T) {
	requireCanonicalize(t, "with-allowance")
}

func TestCanonicalizeReverseChargeFixture(t *testing.T) {
	requireCanonicalize(t, "reverse-charge")
}

func TestUnknownFixtureReturns404(t *testing.T) {
	w := httptest.NewRecorder()
	newRouter().ServeHTTP(w, httptest.NewRequest(http.MethodPost, "/canonicalize/does-not-exist", nil))
	if w.Code != http.StatusNotFound {
		t.Fatalf("expected 404, got %d", w.Code)
	}
	if !strings.Contains(w.Body.String(), "UNKNOWN_FIXTURE") {
		t.Fatalf("expected UNKNOWN_FIXTURE in body, got %s", w.Body.String())
	}
}

func requireCanonicalize(t *testing.T, name string) {
	t.Helper()
	w := httptest.NewRecorder()
	newRouter().ServeHTTP(w, httptest.NewRequest(http.MethodPost, "/canonicalize/"+name, nil))
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200 for %s, got %d (body: %s)", name, w.Code, w.Body.String())
	}
	var body map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if status, _ := body["_engine_status"].(float64); status != 0 {
		t.Fatalf("expected _engine_status=0 for %s, got %v", name, body["_engine_status"])
	}
}
