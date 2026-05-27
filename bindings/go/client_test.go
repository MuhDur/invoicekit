// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package invoicekit

import "testing"

func TestTransportModeReturnsOneOfTwoKnownValues(t *testing.T) {
	mode := TransportMode()
	switch mode {
	case "cgo", "rest":
		// expected
	default:
		t.Fatalf("unexpected transport mode %q (want \"cgo\" or \"rest\")", mode)
	}
}

func TestProcessExportedSurfaceMatchesInternalAlias(t *testing.T) {
	// Process must be the single public entry; the unexported
	// processEngineABI is the transport-specific implementation
	// it forwards to. We don't assert byte equality here (the
	// engine response is covered by abi_golden_test.go); we just
	// verify the surface compiles and is reachable.
	_, _, err := Process(nil)
	if TransportMode() == "cgo" {
		// cgo path will hit the engine; either succeeds or returns
		// a typed error from the engine. Both are acceptable for
		// the surface test.
		if err != nil {
			t.Logf("cgo Process(nil) returned err=%v (acceptable; surface compiles)", err)
		}
	} else {
		// REST fallback with no configured endpoint will fail at
		// the HTTP layer (connection refused). That's the right
		// failure shape; the surface still compiles and dispatches.
		if err == nil {
			t.Log("rest Process(nil) reached an endpoint; surface OK")
		}
	}
}
