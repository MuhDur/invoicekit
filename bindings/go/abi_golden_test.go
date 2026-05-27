// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//go:build cgo

package invoicekit

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

type goldenFixture struct {
	RequestBytes          string `json:"request_bytes"`
	ExpectedResponseBytes string `json:"expected_response_bytes"`
}

func TestEngineABIGoldenFixtureViaCgo(t *testing.T) {
	fixture := readGoldenFixture(t)
	actual, status, err := processEngineABI([]byte(fixture.RequestBytes))
	if err != nil {
		t.Fatal(err)
	}
	if status != 0 {
		t.Fatalf("expected status 0, got %d", status)
	}
	if string(actual) != fixture.ExpectedResponseBytes {
		t.Fatal("Go cgo ABI response did not match golden bytes")
	}
}

func TestEngineABIEmptyRequestReachesEngine(t *testing.T) {
	actual, status, err := processEngineABI(nil)
	if err != nil {
		t.Fatal(err)
	}
	if status != 1 {
		t.Fatalf("expected status 1, got %d", status)
	}
	if !json.Valid(actual) {
		t.Fatal("expected canonical JSON error response")
	}
}

func readGoldenFixture(t *testing.T) goldenFixture {
	t.Helper()
	root := repoRoot(t)
	path := filepath.Join(root, "conformance-corpus", "golden", "engine-abi-v1-commercial-document.json")
	bytes, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var fixture goldenFixture
	if err := json.Unmarshal(bytes, &fixture); err != nil {
		t.Fatal(err)
	}
	return fixture
}

func repoRoot(t *testing.T) string {
	t.Helper()
	workingDirectory, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	return filepath.Clean(filepath.Join(workingDirectory, "..", ".."))
}
