// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Emit the canonical JSON Schema for [`invoicekit_validate::ValidationResult`].
//!
//! Used by the CI gate that re-derives the schema and asserts byte equality
//! against the committed `schemas/validation-result.schema.json`.

fn main() {
    let schema = invoicekit_validate::validation_result_schema();
    let pretty =
        serde_json::to_string_pretty(&schema).expect("schemars output is always serializable");
    println!("{pretty}");
}
