// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Package invoicekit exposes a stable Go SDK over the InvoiceKit
// Rust engine. The package picks one of two transports at
// compile time:
//
//   - cgo (default): links against the libinvoicekit_ffi C ABI.
//     Requires CGO_ENABLED=1 and a build of crates/invoicekit-ffi
//     on the build host's library search path.
//   - REST fallback: when CGO_ENABLED=0 (typical cross-compile or
//     locked-down build environment), the package POSTs Engine
//     ABI JSON to a rest-shim sidecar configured via the
//     INVOICEKIT_REST_URL env var.
//
// Consumers use TransportMode at runtime to verify which
// transport the binary was built with.
package invoicekit

// TransportMode reports the linked transport: "cgo" or "rest".
// Useful in test fixtures that need to skip a particular
// transport's assertions, and in production for the operator
// log emitted by ClientFor.
func TransportMode() string { return transportMode() }

// Process runs an Engine ABI JSON request through the linked
// transport. The request body must be a canonical Engine ABI
// JSON envelope (see crates/invoicekit-ffi/ABI.md):
//
//	{"abi_version": 1, "operation": "...", "payload": {...}}
//
// Returns the raw JSON response bytes, the engine status code
// (0 = success, non-zero = error envelope), and any transport
// error encountered. A non-nil error means the call did not
// reach the engine; a non-zero status with nil error means the
// engine produced an error envelope worth inspecting.
func Process(request []byte) ([]byte, uint32, error) {
	return processEngineABI(request)
}
