// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Fuzz target: `invoicekit_format_ubl::from_xml` must never panic on
//! arbitrary UTF-8 input. Malformed UBL returns a typed `UblError`; the
//! fuzzer only flags panics, infinite loops, and aborts.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = invoicekit_format_ubl::from_xml(text);
    }
});
