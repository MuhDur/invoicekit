// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Emit the canonical JSON Schema for [`invoicekit_validate::ValidationExplainPlan`].
//!
//! Used by T-032a to keep `schemas/validation-explain-plan.schema.json`
//! aligned with the Rust source of truth.

fn main() {
    let schema = invoicekit_validate::validation_explain_plan_schema();
    let pretty =
        serde_json::to_string_pretty(&schema).expect("schemars output is always serializable");
    println!("{pretty}");
}
