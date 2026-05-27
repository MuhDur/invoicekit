// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//go:build !cgo

package invoicekit

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"
)

// T-107: REST fallback for pure-Go contexts (CGO_ENABLED=0, or
// cross-compile targets without the libinvoicekit_ffi shared
// object available). This path makes the same Engine ABI request
// over HTTP to the rest-shim sidecar.
//
// The endpoint is taken from INVOICEKIT_REST_URL (e.g.
// http://localhost:8081). Authentication, if any, is configured
// via INVOICEKIT_REST_BEARER. The endpoint must implement
// POST /v1/engine/process_json that accepts the raw Engine ABI
// JSON body and returns:
//
//	{"status": <uint32>, "response_base64": "<base64 bytes>"}
//
// Tests inject a custom transport via the unexported overrideHTTPClient.

const (
	defaultRestEndpoint = "http://127.0.0.1:8081"
	envRestURL          = "INVOICEKIT_REST_URL"
	envRestBearer       = "INVOICEKIT_REST_BEARER"
	restPath            = "/v1/engine/process_json"
)

// overrideHTTPClient is settable by tests to inject a roundtripper
// without touching the network. Production code leaves it nil.
var overrideHTTPClient *http.Client

type restResponse struct {
	Status         uint32 `json:"status"`
	ResponseBase64 string `json:"response_base64"`
}

func processEngineABI(request []byte) ([]byte, uint32, error) {
	endpoint := os.Getenv(envRestURL)
	if endpoint == "" {
		endpoint = defaultRestEndpoint
	}
	body := bytes.NewReader(request)
	req, err := http.NewRequest(http.MethodPost, endpoint+restPath, body)
	if err != nil {
		return nil, 0, fmt.Errorf("invoicekit rest-shim: build request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	if bearer := os.Getenv(envRestBearer); bearer != "" {
		req.Header.Set("Authorization", "Bearer "+bearer)
	}

	client := overrideHTTPClient
	if client == nil {
		client = &http.Client{Timeout: 30 * time.Second}
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, 0, fmt.Errorf("invoicekit rest-shim: post: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 {
		buf, _ := io.ReadAll(resp.Body)
		return nil, 0, fmt.Errorf("invoicekit rest-shim: HTTP %d: %s", resp.StatusCode, string(buf))
	}
	var parsed restResponse
	if err := json.NewDecoder(resp.Body).Decode(&parsed); err != nil {
		return nil, 0, fmt.Errorf("invoicekit rest-shim: decode response: %w", err)
	}
	decoded, err := decodeBase64(parsed.ResponseBase64)
	if err != nil {
		return nil, 0, fmt.Errorf("invoicekit rest-shim: decode response bytes: %w", err)
	}
	if len(decoded) == 0 {
		return nil, parsed.Status, errors.New("invoicekit rest-shim: empty response")
	}
	return decoded, parsed.Status, nil
}

func transportMode() string { return "rest" }
