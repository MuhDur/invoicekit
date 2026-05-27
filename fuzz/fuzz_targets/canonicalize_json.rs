// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Fuzz target: the RFC 8785 JSON canonicalizer must never panic or
//! deadlock on arbitrary UTF-8 input. Wrong-shaped JSON returns a typed
//! `CanonicalizeError`; the fuzzer only flags panics, infinite loops,
//! and aborts.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = invoicekit_canonical::canonicalize(text);
    }
});
