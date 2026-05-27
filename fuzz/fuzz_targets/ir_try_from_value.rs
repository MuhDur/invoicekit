// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Fuzz target: `CommercialDocument::try_from_value` must never panic on
//! arbitrary JSON. Wrong-shaped input returns a typed `IrError`; the
//! fuzzer only flags panics, infinite loops, and aborts.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            let _ = invoicekit_ir::CommercialDocument::try_from_value(value);
        }
    }
});
