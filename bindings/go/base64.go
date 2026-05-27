// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package invoicekit

import "encoding/base64"

// decodeBase64 wraps stdlib base64.StdEncoding.DecodeString so
// callers don't need to import encoding/base64 directly. It is
// used by the REST fallback to unwrap the engine response.
func decodeBase64(s string) ([]byte, error) {
	if s == "" {
		return nil, nil
	}
	return base64.StdEncoding.DecodeString(s)
}
