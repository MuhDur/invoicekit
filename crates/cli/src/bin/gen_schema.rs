// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Emit the canonical JSON Schema for [`invoicekit_ir::CommercialDocument`].
//!
//! T-011 deliverable. Run with `cargo run --bin gen-schema -p
//! invoicekit-cli > schemas/invoicekit-ir-v1.json` and commit the result;
//! CI re-derives and diffs on every PR via
//! `tools/release-checks/test_ir_schema_match.py`.

fn main() {
    let schema = invoicekit_ir::commercial_document_schema();
    let pretty =
        serde_json::to_string_pretty(&schema).expect("schemars output is always serializable");
    println!("{pretty}");
}
