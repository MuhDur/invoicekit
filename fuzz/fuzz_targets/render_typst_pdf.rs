// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Fuzz target (oueo): drive the Typst compiler + PDF exporter through
//! the `render_for_fuzz` shim. Adversarial input is expected to fail
//! Typst compilation most of the time — the fuzzer only flags panics,
//! deadlocks, aborts, and out-of-memory crashes. Typed
//! `RenderPdfError` returns are normal.
//!
//! Typst compilation costs 10-100 milliseconds per iteration, so this
//! target runs as a nightly higher-iteration job rather than the
//! 5-minute PR matrix (`.github/workflows/fuzz-nightly.yml`).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        let _ = invoicekit_render_pdf::render_for_fuzz(source);
    }
});
