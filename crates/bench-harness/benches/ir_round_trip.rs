//! Tracked operation: `ir-round-trip`.
//!
//! Measures the cost of decoding a representative commercial document from a
//! `serde_json::Value` and re-encoding it back, exercising the full
//! `invoicekit-ir` validation + (de)serialization path. The T-007 budget
//! workflow compares the criterion estimate for this operation against the
//! rolling baseline published from `main`; a regression beyond the threshold
//! configured in `tools/perf-budget/budget.toml` (default 10%) fails the PR.

// criterion_group! / criterion_main! expand to public items without rustdoc;
// the workspace's missing_docs warning would otherwise fail clippy here.
#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_ir::CommercialDocument;
use serde_json::{json, Value};

fn synthetic_document() -> Value {
    json!({
        "schema_version": "1.0",
        "id": "doc_bench_0001",
        "document_type": "invoice",
        "issue_date": "2026-05-26",
        "due_date": "2026-06-25",
        "document_number": "INV-BENCH-0001",
        "currency": "EUR",
        "supplier": party_json("supplier-1", "Bench Supplier", "DE"),
        "customer": party_json("customer-1", "Bench Customer", "FR"),
        "payment_terms": {
            "description": "30 days net",
            "due_date": "2026-06-25"
        },
        "payment_instructions": [{
            "kind": "iban_bic",
            "account": "DE02100100100006820101",
            "reference": "INV-BENCH-0001"
        }],
        "lines": [{
            "id": "1",
            "description": "Bench line item",
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
        "meta": {
            "tenant_id": "tenant_bench",
            "trace_id": "trace_bench"
        }
    })
}

fn party_json(id: &str, name: &str, country: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "tax_ids": [{ "scheme": "vat", "value": "0000000000" }],
        "address": {
            "lines": ["Bench Street 1"],
            "city": "Bench City",
            "postal_code": "00000",
            "country": country
        }
    })
}

fn bench_ir_round_trip(c: &mut Criterion) {
    let input = synthetic_document();
    c.bench_function("ir-round-trip", |b| {
        b.iter(|| {
            let doc = CommercialDocument::try_from_value(black_box(input.clone())).unwrap();
            let out = doc.to_value().unwrap();
            black_box(out);
        });
    });
}

criterion_group!(benches, bench_ir_round_trip);
criterion_main!(benches);
