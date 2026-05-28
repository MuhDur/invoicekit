// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operation: `render-pdf`.
//!
//! Measures `invoicekit-render-pdf` rendering one validated commercial document
//! to deterministic PDF/A-3b bytes. This is the user-facing render verb and was
//! previously unbenchmarked.
//!
//! The dominant cost today is document-independent: every call to the internal
//! `InMemoryWorld::new` rebuilds the full Typst standard library and re-indexes
//! the embedded font set (`crates/render-pdf/src/lib.rs:362`). Because that
//! fixed cost dwarfs the per-line layout work for a small invoice, this bench
//! deliberately renders a small (single-line) document — the regime where the
//! setup cost is most visible and where the rank-1 optimization (hoist the
//! library + fonts into a `LazyLock`) will show up most clearly.
//!
//! Typst rendering is heavier than the parse/validate benches, so this target
//! uses a reduced sample size to keep the harness runtime bounded; the
//! criterion estimate is still stable enough for the regression budget.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_ir::CommercialDocument;
use invoicekit_render_pdf::render_commercial_document_invoice;
use serde_json::{json, Value};

fn sample_document() -> Value {
    json!({
        "schema_version": "1.0",
        "id": "doc_render_bench_0001",
        "document_type": "invoice",
        "issue_date": "2026-05-26",
        "due_date": "2026-06-25",
        "document_number": "INV-RENDER-BENCH-0001",
        "currency": "EUR",
        "supplier": party_json("supplier-1", "Bench Supplier", "DE"),
        "customer": party_json("customer-1", "Bench Customer", "FR"),
        "payment_terms": { "description": "30 days net", "due_date": "2026-06-25" },
        "payment_instructions": [{
            "kind": "iban_bic",
            "account": "DE02100100100006820101",
            "reference": "INV-RENDER-BENCH-0001"
        }],
        "lines": [{
            "id": "1",
            "description": "Render bench line item",
            "quantity": "1",
            "unit_code": "EA",
            "unit_price": "100.00",
            "line_extension_amount": "100.00",
            "tax_category": "S",
            "extensions": []
        }],
        "tax_summary": [{
            "category_code": "S",
            "taxable_amount": "100.00",
            "tax_amount": "19.00",
            "tax_rate": "19.00"
        }],
        "monetary_total": {
            "line_extension_amount": "100.00",
            "tax_exclusive_amount": "100.00",
            "tax_inclusive_amount": "119.00",
            "payable_amount": "119.00"
        },
        "extensions": [],
        "meta": { "tenant_id": "tenant_bench", "trace_id": "trace_bench" }
    })
}

fn party_json(id: &str, name: &str, country: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "tax_ids": [{ "scheme": "vat", "value": format!("{country}123456789") }],
        "address": {
            "lines": ["Bench Street 1"],
            "city": "Bench City",
            "postal_code": "00000",
            "country": country
        }
    })
}

fn bench_render_pdf(c: &mut Criterion) {
    let document = CommercialDocument::try_from_value(sample_document())
        .expect("bench fixture must be a valid commercial document");

    // Warm sanity check: the render path must succeed and emit PDF bytes.
    let pdf = render_commercial_document_invoice(&document).expect("render must succeed");
    assert!(pdf.starts_with(b"%PDF-"));

    // A plain `bench_function` (not a group) keeps the criterion output at
    // `target/criterion/render-pdf/`, which is where the perf-budget tool reads
    // `[operations.render-pdf]`. The reduced sample size is configured on the
    // `Criterion` instance via `criterion_group!` below.
    c.bench_function("render-pdf", |b| {
        b.iter(|| {
            let pdf = render_commercial_document_invoice(black_box(&document)).unwrap();
            black_box(pdf);
        });
    });
}

// Typst compilation costs tens of milliseconds per render, so keep the sample
// count and measurement window modest. The CI bench job may further override
// these via `--sample-size` / `--measurement-time`.
fn render_pdf_criterion() -> Criterion {
    Criterion::default()
        .sample_size(30)
        .measurement_time(Duration::from_secs(12))
}

criterion_group! {
    name = benches;
    config = render_pdf_criterion();
    targets = bench_render_pdf
}
criterion_main!(benches);
