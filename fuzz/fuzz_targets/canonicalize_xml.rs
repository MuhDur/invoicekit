// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Fuzz target: the InvoiceKit XML canonicalizer must never panic on
//! arbitrary UTF-8 input. Malformed XML returns a typed
//! `XmlCanonicalizeError`; the fuzzer only flags panics, infinite loops,
//! and aborts.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = invoicekit_canonical::canonicalize_xml(text);
    }
});
